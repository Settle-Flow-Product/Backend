# 15 · Bằng chứng

> **Bản dịch tiếng Việt** của [`docs/15-evidence.md`](../../15-evidence.md).
> Bản tiếng Anh là bản có hiệu lực.

Nộp gì, và điều kiện để mỗi deliverable được coi là hoàn thành.

Làm xong mọi module mà đến buổi nghiệm thu không có tx hash thì vẫn bị đánh là thiếu bằng chứng. Đó
là rủi ro nghiệm thu tập trung ở nhóm module G, và là lý do trang evidence là một **màn hình** chứ
không phải một bảng tính ai đó điền sau.

## Bằng chứng theo mục 6.1 của SOW

| Deliverable | Loại bằng chứng | Cụ thể |
|---|---|---|
| **1** | Public repo + contract address | Mã nguồn Payment Request Registry và contract ID đã deploy trên Stellar testnet, kèm output `cargo test` |
| **2** | Repo + transaction hashes | Mã nguồn dịch vụ thu chữ ký, và một transaction đã submit thể hiện việc phê duyệt dựa trên threshold |
| **3** | Transaction hashes trên Stellar Expert | Vòng tròn đầy đủ từ lệnh chi tới payout, với link giao dịch on-chain kiểm chứng được |

## Bảy mốc

Điền **trong** buổi demo, không phải sau đó. Bản sống nằm ở
[`.deploy/evidence.md`](../../../.deploy/evidence.md) và được commit có chủ đích — một reviewer mở
repository phải tìm thấy chúng mà không cần chạy gì.

| # | Sự kiện | Kiểm chứng bằng |
|---|---|---|
| **E1** | Deploy registry contract | Stellar Expert · contract id |
| **E2** | Treasury: 3 signer, threshold 2, master 0 | Horizon `/accounts`, so với output script B5 |
| **E3** | `create_request` trên registry | Stellar Expert · tx hash |
| **E4** | `approve` sau khi đủ 2/3 chữ ký | Stellar Expert · tx hash |
| **E5** | Payment tới địa chỉ vendor | Stellar Expert · tx hash |
| **E6** | `mark_paid`, và `get_request` trả `Paid` | Stellar Expert + đọc qua RPC |
| **E7** | Nhánh reject: không transaction nào rời treasury | Khẳng định phủ định — log và test, kèm giải thích cho reviewer |

**E7 là một khẳng định phủ định và cần được xử lý như vậy.** "Không có transaction nào được gửi"
không thể trình bày bằng một đường link, nên bằng chứng là chính bộ test cộng với một truy vấn danh
sách transaction của treasury trong khoảng thời gian liên quan, kèm một câu giải thích reviewer đang
nhìn cái gì. Một reviewer không hiểu một sự vắng mặt chứng minh điều gì sẽ đọc nó thành một lỗ hổng.

**E2 không phải một tx hash** mà là trạng thái tài khoản. Output của script xác minh B5 là thứ được
nộp, vì nó **assert** cấu hình chứ không hiển thị — bao gồm `masterWeight = 0`, thứ reviewer kiểm
đầu tiên.

## Gói nộp

| Hạng mục | Ở đâu |
|---|---|
| Public repo, đã dọn secret, có LICENSE | Git |
| Contract ID testnet của registry | Ghim trong `README.md` gốc |
| Output `cargo test` | `.deploy/coverage.txt` và CI |
| Bảy mốc | `.deploy/evidence.md` và `/evidence/:id` |
| Hướng dẫn deploy | [13-deployment.md](13-deployment.md) và `README.md` |
| Video demo 3–5 phút | Link từ README |

## Definition of Done

### Bắt buộc có

- Test tự động cho happy path và ít nhất một nhánh lỗi.
- Mọi thay đổi trạng thái đều ghi một dòng `audit_events`.
- Mọi endpoint chạm chain trả `202`, không phải `200`.
- Log có `trace_id` nối được từ giao diện tới chain event.
- Mọi migration chạy được cả lên và xuống.
- Script deploy chạy lại được từ đầu dưới mười phút.

### Từ chối merge nếu

- Có `f32` hoặc `f64` ở bất kỳ đâu trong đường tiền.
- Dùng đồng hồ server để quyết định hết hạn.
- Có đường nào hiện "thành công" trước khi readback.
- Có retry tự động mà không qua readback.
- Registry contract xuất hiện lời gọi chuyển token.
- Secret hoặc seed phrase xuất hiện trong repo hay trong log.

## Nghiệm thu

SOW 1 hoàn thành khi một reviewer, không cần ai hỗ trợ, có thể:

1. Mở repository và tìm thấy contract id.
2. Mở Stellar Expert và thấy registry contract đã deploy.
3. Đọc tài khoản treasury từ Horizon và xác nhận 3 signer, threshold 2, master weight 0.
4. Đi theo E3 → E4 → E5 → E6 và xem một lệnh chi từ lúc tạo tới lúc paid.
5. Đọc giải thích nhánh từ chối và hiểu vì sao không có transaction nào cho nó.
6. Clone repository và deploy lại toàn bộ từ đầu.

Mục 6 là mục hay bị bỏ nhất, và là mục biến một buổi demo thành một deliverable.

## Những quy tắc tài liệu này áp đặt

- **Hash được thu ngay khi phát sinh.** Hash không ghi ngay lúc đó là hash mất luôn.
- **`.deploy/evidence.md` được commit**, không phải sinh ra khi cần.
- **E7 nộp kèm một giải thích**, vì một sự vắng mặt tự nó không chứng minh gì.
- **Hướng dẫn deploy được kiểm bởi người không viết nó.**
- **Dọn secret trước khi công khai repository** — kiểm, chứ không giả định.
