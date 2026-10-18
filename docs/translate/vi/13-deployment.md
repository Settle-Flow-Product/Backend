# 13 · Triển khai

> **Bản dịch tiếng Việt** của [`docs/13-deployment.md`](../../13-deployment.md).
> Bản tiếng Anh là bản có hiệu lực.

Module G1 và G4. Build, deploy, cấu hình, xác minh — và làm lại được toàn bộ từ con số không dưới
mười phút.

## Đọc trước

- **Testnet reset.** Khi nó reset, contract đã deploy và mọi transaction demo mất theo. Đó là lý do
  việc dựng lại trong mười phút là một yêu cầu chứ không phải một sự gọn gàng — nó là khác biệt
  giữa mất một buổi chiều và mất một deliverable.
- **Không thao tác tay.** Không một địa chỉ nào gõ bằng tay, không một cấu hình nào đặt qua UI. Một
  bước do người làm là một bước bị bỏ qua dưới áp lực của buổi demo, và `masterWeight = 0` chính là
  bước bị bỏ qua.
- **Chỉ testnet.** Mainnet nằm ngoài phạm vi.
- **Ghim SDK ngay Ngày 1.** `Cargo.lock` được commit và không update giữa chặng.

## Build

```bash
make -C smart-contract build      # fmt, clippy -D warnings, test, rồi wasm release
make -C backend all               # fmt, clippy, test, build
cd frontend/website && npm ci && npm run build
```

Build contract chạy theo release profile — `opt-level = "z"`, LTO, strip symbol. Profile đó chỉ có
hiệu lực ở workspace root, đó là lý do contract và backend là **hai Cargo workspace tách biệt** và
không bao giờ gộp.

## Deploy

```bash
make -C smart-contract deploy KEY=deployer NETWORK=testnet
```

`scripts/deploy.sh` deploy và cấu hình contract **trong cùng một transaction** —
`stellar contract deploy … -- --admin … --settler …`, lệnh này gọi `__constructor` của contract.

Contract cố ý **không có** entry point `initialize`. Một contract deploy ở một transaction rồi cấu
hình ở transaction khác có một cửa sổ giữa hai bước mà bất kỳ ai cũng có thể cấu hình nó. Ở đây
không có cửa sổ đó, và script đọc lại cấu hình từ chain trước khi ghi bất cứ gì vào `.deploy/env`.

Nó ghi `.deploy/env`, chỉ chứa địa chỉ, không bao giờ chứa secret.

## Cấu hình treasury

```bash
./backend/scripts/setup_treasury.sh     # B1 + B2: fund, trustline, 3 signer, threshold, master 0
./backend/scripts/verify_treasury.sh    # B5: đọc lại từ Horizon và ASSERT
```

`verify_treasury.sh` **assert** chứ không in. Một script in cấu hình rồi để người kiểm là một script
pass trong buổi demo trong khi master key vẫn còn weight 1. Output của nó là mốc bằng chứng **E2**.

## Chạy stack

```bash
make dev           # postgres, backend, website, admin
make migrate       # áp dụng mọi migration
```

| Dịch vụ | Cổng | Ghi đè bằng |
|---|---|---|
| backend | 8088 → 8080 (`/health`, `/ready`) | `BACKEND_PORT` |
| website | 5173 | `WEBSITE_PORT` |
| admin | 5174 | `ADMIN_PORT` |
| postgres | 5432 | `POSTGRES_PORT` |
| adminer | 8081 (compose profile `sql`) | `ADMINER_PORT` |

Chỉ phía host của mỗi ánh xạ là ghi đè được. Chúng tồn tại vì một stack thứ hai trên cùng một máy
không nên chết chỉ vì một số cổng — một lập trình viên đang chạy Postgres của dự án khác trên 5432
là chuyện bình thường, không phải lỗi cấu hình.

## Biến môi trường

Backend đọc mọi thứ từ môi trường lúc khởi động. Một sub-struct `Option` nghĩa là hệ thống con đó
tắt; cấu hình nửa vời là một lỗi khởi động chứ không phải một bất ngờ lúc chạy.

| Biến | Mục đích | Mặc định |
|---|---|---|
| `DATABASE_URL` | Postgres | **bắt buộc** |
| `BIND` | Địa chỉ lắng nghe | `0.0.0.0:8080` |
| `SOROBAN_RPC_URL` | Soroban RPC | testnet |
| `HORIZON_URL` | Horizon | testnet |
| `NETWORK_PASSPHRASE` | Gắn network | `Test SDF Network ; September 2015` |
| `REGISTRY_CONTRACT_ID` | Registry đã deploy | **bắt buộc** |
| `TREASURY_ADDRESS` | G-account các lệnh chi rút từ | **bắt buộc** |
| `BUSINESS_KEY` / `SETTLER_KEY` | **Tên khoá** trong keystore của Stellar CLI | **bắt buộc** |
| `SESSION_KEY` | Ký session token của website | **bắt buộc** |
| `TX_TIMEBOUNDS_SECONDS` | Cửa sổ ký | `3600` |
| `CHALLENGE_TTL_SECONDS` | Hạn của challenge | `300` |

`SESSION_KEY`, `BUSINESS_KEY` và `SETTLER_KEY` **không bao giờ có mặc định**. Một secret mặc định là
một secret trong production.

### Hai ứng dụng frontend

| Biến | Được đọc bởi | Mục đích |
|---|---|---|
| `API_TARGET` | website | Backend mà nó proxy tới, ở phía server |
| `DATABASE_URL` | admin | Bảng điều khiển đọc Postgres trực tiếp — xem [11-admin.md](11-admin.md) |
| `DATABASE_POOL_MAX` | admin | Số kết nối bảng điều khiển được giữ (`6`) |
| `ADMIN_SESSION_KEY` | admin | Ký cookie phiên của bảng điều khiển. Là giá trị **khác** `SESSION_KEY`: token phát cho ứng dụng này không được mở ứng dụng kia |
| `COOKIE_SECURE` | admin | `true` khi chạy sau TLS |

**Không ứng dụng nào có biến `NEXT_PUBLIC_*`**, nên một image chạy được với mọi môi trường. Với bảng
điều khiển, đó còn là một thuộc tính bảo mật: một database URL nhúng vào lúc build là một thông tin
đăng nhập nằm sẵn trong bundle của trình duyệt.

Tệp cấu hình của cả hai ứng dụng đều không phải `.ts`. Next gọi tới trình biên dịch TypeScript để
nạp `next.config.ts` lúc khởi động, mà image production không có devDependencies để tìm ra trình
biên dịch đó — nên cả hai dùng `next.config.mjs`.

## Thứ tự khởi động

Thứ tự khởi động của backend mang tính quyết định và fail sớm:

```
1  kết nối Postgres và chạy migration
2  kiểm tra registry trả lời và treasury tồn tại
3  dựng chain client
4  spawn indexer và submitter
5  bind cổng
6  serve, có graceful shutdown
```

Bước 2 đứng **trước** bước 5 một cách có chủ đích. Một backend bind cổng rồi mới phát hiện không với
tới chain sẽ tự báo là khoẻ trong khi vô dụng, và load balancer sẽ tin nó.

## Checklist sau deploy

- [ ] `verify_treasury.sh` pass mọi khẳng định.
- [ ] `get_request` trên một lệnh chi vừa tạo trả `Pending` qua RPC simulation.
- [ ] Contract id đã ghim vào `README.md` gốc.
- [ ] `.deploy/evidence.md` có contract id và mọi hash đã thu.
- [ ] `curl localhost:8088/ready` trả 200.
- [ ] `make verify-wallet-auth` pass — một chữ ký SEP-10 thật được chấp nhận, còn nonce dùng lại và
      chữ ký từ khoá khác đều bị từ chối.
- [ ] Website đăng nhập được bằng ví và liệt kê được lệnh chi.
- [ ] Bảng điều khiển quản trị đăng nhập được bằng tài khoản tạo qua `make admin-user`, liệt kê được
      một bảng, và trả 404 cho `/admin/admin_users`.
- [ ] Không secret nào xuất hiện trong repository hay trong log.
- [ ] Toàn bộ trình tự đã chạy lại từ đầu và bấm giờ dưới mười phút.

## Những quy tắc tài liệu này áp đặt

- **Không thao tác tay.** Nếu một người phải gõ một địa chỉ, script chưa xong.
- **Deploy và cấu hình trong một transaction**, qua `__constructor`.
- **Xác minh phải assert**, không phải in ra.
- **`Cargo.lock` ghim ngay Ngày 1** và không update giữa chặng.
- **Secret không bao giờ có mặc định** và không bao giờ được log.
- **Fail trước khi bind cổng**, không phải sau.
- **Việc dựng lại được bấm giờ**, và mười phút là con số.
