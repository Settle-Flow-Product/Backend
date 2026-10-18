# 04 · Giao diện contract

> **Bản dịch tiếng Việt** của [`docs/04-contract-interface.md`](../../04-contract-interface.md).
> Bản tiếng Anh là bản có hiệu lực.

`payment_request_registry` — contract Soroban cho Deliverable 1. Nó ghi nhận và xác minh vòng đời
của một lệnh chi. Nó không bao giờ chạm vào token.

Đây là giao diện cứng. **Thứ tự gate là một phần của đặc tả, không phải chi tiết cài đặt.** Test
khẳng định mã lỗi chính xác, nên đảo hai gate là một thay đổi phá vỡ tương thích ngay cả khi cả hai
vẫn từ chối.

## Vì sao contract này lên được testnet mà không cần audit

Không có hàm nào ở đây chuyển được tài sản. Không `token::Client`, không `transfer`, không lời gọi
nào tới Stellar Asset Contract trong toàn bộ crate. Bề mặt rủi ro custody bằng không.

Tính chất đó được kiểm bằng máy, không phải bằng trí nhớ. Đây là **mục đầu tiên trong checklist
review Ngày 4**, và CI thực thi nó ở mỗi lần push:

```bash
# Bất kỳ kết quả nào cũng làm fail build.
grep -rn -e 'token::Client' -e 'TokenClient' -e '\.transfer(' smart-contract/src/ && exit 1
```

## Ma trận hàm và quyền

| Hàm | Ai gọi được | Điều kiện | Chạm tài sản |
|---|---|---|---|
| `__constructor` | Người deploy, ngay trong transaction deploy | Chạy đúng một lần, theo cấu trúc. Bump TTL instance ngay tại đây | Không |
| `create_request` | Doanh nghiệp, cho chính mình | `amount > 0`, expiry ở tương lai, id chưa tồn tại | Không |
| `approve` | Chỉ settler | Đang `Pending`, chưa quá hạn. Backend gọi sau khi đã thu đủ weight | Không |
| `mark_paid` | Chỉ settler | Chưa có `paid_tx`, đang `Approved`, chưa quá hạn, amount khớp | Không |
| `cancel_request` | Doanh nghiệp tạo lệnh | Chưa `Paid` | Không |
| `get_request` · `is_payable` | Bất kỳ ai — đọc qua RPC simulation | — | Không |

`get_request` và `is_payable` là hàm đọc, nhưng Soroban **không có view function** — mọi entry point
đều đi qua `InvokeHostFunction`. Chúng chỉ miễn phí khi gọi off-chain bằng `simulateTransaction`
hoặc `getLedgerEntries`. **Đừng bao giờ submit chúng như một transaction.**

## Kiểu dữ liệu

```rust
// SOW nêu đích danh ba trạng thái: pending → approved → paid.
pub enum ReqStatus { Pending = 0, Approved = 1, Paid = 2, Cancelled = 3 }

pub struct PaymentRequest {
    pub id:        BytesN<32>,   // sha256(business_id ‖ idempotency_key)
    pub creator:   Address,      // doanh nghiệp tạo lệnh — bên có quyền huỷ
    pub payee:     Address,      // ĐỊA CHỈ VENDOR, theo đúng SOW Deliverable 1
    pub asset:     Address,      // SAC của USDC, hoặc của XLM native
    pub amount:    i128,         // stroops · 7 thập phân · không bao giờ float
    pub reference: BytesN<32>,   // HASH của mã tham chiếu; chuỗi thô không lên chain
    pub expiry:    u64,          // ledger timestamp, không phải đồng hồ server
    pub status:    ReqStatus,
    pub paid_tx:   Option<BytesN<32>>,
}
```

Chín trường, không hơn. Mỗi trường thừa trên chain là storage rent không ai dự trù.

Ba trường đáng lập luận riêng:

**`payee` là vendor.** Creator là doanh nghiệp chi tiền; payee là nhà cung cấp nhận tiền. Đây là
trường khiến contract trở thành công cụ accounts payable chứ không phải receivable, và là trường
SOW nêu đích danh.

**`reference` là một hash.** Chuỗi thô là mã đơn hàng hoặc số hoá đơn — nhạy cảm về thương mại, và
sẽ công khai vĩnh viễn nếu ghi lên ledger. Chuỗi thô nằm ở Postgres; chain giữ `sha256` của nó, đủ
để chứng minh tương ứng khi đối soát.

**`expiry` là ledger timestamp.** Nó được so với `env.ledger().timestamp()` và không gì khác.

## Cấu hình xảy ra trong constructor

Contract **không có** entry point `initialize`, và đó là chủ đích. Một contract deploy ở một
transaction rồi cấu hình ở một transaction khác có một cửa sổ giữa hai bước, trong đó bất kỳ ai cũng
có thể cấu hình nó. Host chạy `__constructor` như một phần của `create_contract`, nên cửa sổ đó
không tồn tại.

Đó cũng là lý do `AlreadyInitialized`, `NotInitialized` và `SettlerNotSet` là **mã dự trữ và không
thể xảy ra** thay vì mã sống: điều kiện mà mỗi mã đó mô tả giờ không biểu diễn được. Số của chúng
vẫn bị chiếm, vì một số mã lỗi không bao giờ được dùng lại.

Lỗi này được phát hiện bằng cách deploy thật. Bản trước phơi ra `initialize` và trông cậy vào
`stellar contract deploy … -- --admin … --settler …` để gọi nó; CLI gọi `__constructor`, nên các
tham số được nhận và bỏ qua, và contract lên testnet ở trạng thái chưa cấu hình. Mọi lời gọi trả về
`NotInitialized` cho tới khi được sửa.

## Mã lỗi

Đánh số theo dải chục để dải của một mã cho biết hệ thống con nào từ chối. **Một mã không bao giờ
đổi số sau lần deploy testnet công khai đầu tiên**, và mã bị bỏ để lại số của nó trống vĩnh viễn.

| Dải | Mã | Tên | Khi nào |
|---|---|---|---|
| Hệ thống 1–9 | 1 | `AlreadyInitialized` | `initialize` gọi lần thứ hai |
| | 2 | `NotInitialized` | Gọi trước khi `initialize` |
| | 3 | `SettlerNotSet` | Thiếu settler trong instance storage |
| | 4–9 | *(trống)* | Dự trữ |
| Request 10–19 | 10 | `RequestExists` | `create_request` với id đã có trong persistent storage |
| | 11 | `RequestNotFound` | Không có entry cho id này — chưa từng tồn tại |
| | 12 | `InvalidAmount` | `amount <= 0` lúc tạo, hoặc lệch amount lúc `mark_paid` |
| | 13 | `InvalidExpiry` | `expiry <= env.ledger().timestamp()` lúc tạo |
| | 14–19 | *(trống)* | Dự trữ |
| Vòng đời 20–29 | 20 | `NotPending` | Một transition đòi `Pending` nhưng gặp trạng thái khác |
| | 21 | `NotApproved` | `mark_paid` trên request không phải `Pending` cũng không phải `Approved` |
| | 22 | `AlreadyPaid` | `paid_tx` đã được đặt |
| | 23 | `Expired` | `env.ledger().timestamp() >= expiry` |
| | 24–29 | *(trống)* | Dự trữ |
| Quyền 30–39 | 30 | `Unauthorized` | Dự trữ cho kiểm tra quyền mà host chưa panic sẵn |

Đúng 12 mã, khớp module **A1**.

`require_auth` thất bại thì **panic trong host** chứ không trả mã lỗi — đó là hành vi của Soroban,
không phải lựa chọn. Test cho những ca đó khẳng định panic, không khẳng định mã.

`Error` derive `Debug`. Đây không phải chuyện thẩm mỹ: thiếu nó thì `.unwrap()` trên
`Result<_, Error>` không compile trong test, và toàn bộ lối khẳng định mã chính xác biến mất.

## Storage và TTL — module A8

```rust
const BUMP_THRESHOLD: u32 = 518_400;   // ~30 ngày NẾU ledger đóng mỗi 5 giây
const BUMP_AMOUNT:    u32 = 725_760;   // ~42 ngày NẾU ledger đóng mỗi 5 giây
```

Cả hai đếm bằng **ledger**, không phải giây. Thời gian đóng ledger là tham số mạng và có thể giảm;
ở 4 giây, "42 ngày" thành khoảng 33,6. Tính lại lúc build thay vì chép con số này đi tiếp.

| Key | Độ bền | Được gia hạn bởi |
|---|---|---|
| `Admin`, `Settler` | Instance | **Mọi entry point** |
| `Req(id)` | Persistent | Mọi hàm đọc hoặc ghi request đó |

**Instance storage dùng chung TTL với mã Wasm, và nếu nó hết hạn thì mọi hàm đều fail** cho tới khi
có `RestoreFootprint`. Đó là lý do A8 nói phải gia hạn instance TTL ở mọi entry point, không chỉ ở
những hàm ghi.

**Entry bị archive không giống entry không tồn tại.** Transaction chạm một key đã archive sẽ fail
lúc apply, trước khi tới contract — nên `get_request` không trả `RequestNotFound` và `is_payable`
không trả `false`. Nó abort. Backend phải phân biệt hai ca, vì chỉ ca thứ hai phục hồi được.

## Kỷ luật ghi

Mọi entry point có thay đổi trạng thái đều theo cùng thứ tự:

```
 1  require_auth đúng vai
 2  validate tham số
 3  load state
 4  guards, rẻ trước đắt sau
 5  áp dụng thay đổi
 6  phát event
 7  gia hạn TTL          luôn cuối cùng, luôn có giới hạn
```

Thứ tự gate của từng hàm, và event, xem bản tiếng Anh
[`docs/04-contract-interface.md`](../../04-contract-interface.md).

## Event — module A7

Topic đầu luôn là `"settle"`. Topic thứ hai được rút gọn để vừa 9 ký tự, giới hạn của `Symbol` dạng
ngắn — giới hạn này làm người ta bất ngờ đúng vào lúc tệ nhất.

| Topic | Data | Phát bởi |
|---|---|---|
| `settle` · `req_new` · id | `(amount, expiry)` | `create_request` |
| `settle` · `req_appr` · id | `()` | `approve` |
| `settle` · `req_paid` · id | `(amount, tx)` | `mark_paid` |
| `settle` · `req_canc` · id | `()` | `cancel_request` |

Ba tính chất mọi event đều có và giữ nguyên: nó nêu tên request của mình ở topic, nên indexer lọc
được mà không cần decode data; các trường không bao giờ đổi sau khi deploy, vì đang có indexer chạy
production parse chúng; và nó được phát ở bước 6, sau khi đã ghi state, không bao giờ trước.

## Những quy tắc tài liệu này áp đặt

- **Không hàm nào trong crate này được chuyển token.** Thực thi bằng grep gate trong CI.
- **Thứ tự gate là đặc tả.** Test khẳng định mã chính xác; đảo gate là thay đổi phá vỡ tương thích.
- **Số mã lỗi đóng băng ở lần deploy công khai đầu tiên.** Số đã bỏ để trống vĩnh viễn.
- **Thời gian ledger quyết định hết hạn.** `env.ledger().timestamp()`, không bao giờ đồng hồ server.
- **Mọi entry point gia hạn instance TTL**, không chỉ entry mà nó chạm.
- **Archive không phải là không tồn tại.** Backend xử lý hai ca khác nhau.
