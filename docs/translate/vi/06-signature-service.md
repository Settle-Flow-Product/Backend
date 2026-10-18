# 06 · Dịch vụ thu chữ ký

> **Bản dịch tiếng Việt** của [`docs/06-signature-service.md`](../../06-signature-service.md).
> Bản tiếng Anh là bản có hiệu lực.

Module C1–C8 — Deliverable 2, phần hai. Dịch vụ dựng một payment Stellar chưa ký từ lệnh chi đã
approved, giữ sequence number của treasury trong lúc approver ký, verify từng chữ ký, và submit khi
tổng weight đạt ngưỡng.

Đây là nơi chứa giá trị của SOW 1, và cũng là nơi một ngoại lệ xử lý sai chi cho vendor hai lần.

## C1 · Dựng transaction chưa ký

| Thành phần | Giá trị | Vì sao |
|---|---|---|
| Source | Treasury G-account | Nó cấp sequence number và chữ ký |
| Operation | `payment(payee, asset, amount)` | Lấy thẳng từ lệnh chi đã approved |
| Memo | `Memo::hash(request_id)` | **Khoá tra ngược.** Thiếu nó, C6 không tìm được một transaction mà ta chưa từng nhận được hash |
| Timebounds | một giờ | Một transaction đã ký chưa submit là một tấm séc đã ký |
| Fee | base fee, bump sau nếu cần | Xem C7 — không bao giờ dựng lại vì lý do phí |

Tx hash trả về chính là thứ approver ký. **Nó đổi nếu sequence number đổi**, đó là lý do sequence
được giữ trước khi tính hash và giữ cho tới khi settlement kết thúc.

## C2 · Khoá sequence — feature F-09

Một lệnh chi đang bay mỗi treasury, thực thi bằng database chứ không bằng sự cẩn thận.

```sql
CREATE UNIQUE INDEX settlements_one_in_flight_per_treasury
    ON settlements (treasury_address)
    WHERE status IN ('awaiting_approval', 'releasing', 'unresolved');
```

**Khoá theo TÀI KHOẢN TREASURY, không phải theo lệnh chi và không phải theo tài sản.** Một lệnh chi
USDC và một lệnh chi XLM cùng treasury vẫn tranh chấp cùng một sequence number. Khoá theo bất cứ thứ
gì hẹp hơn sẽ tái tạo đúng lỗi mà nó sinh ra để ngăn — và chỉ tái tạo dưới điều kiện đồng thời, tức
là ở buổi demo.

Lệnh thứ hai nhận `409 RELEASE_IN_FLIGHT` ngay lập tức. Đó chính là điểm mấu chốt: bị từ chối sạch
sẽ từ đầu, thay vì được chấp nhận, thu chữ ký của hai người, rồi chết vì `tx_bad_seq`.

## C3 · Nhận một chữ ký — feature F-10

Ba kiểm tra, ba loại thất bại khác nhau:

| Kiểm tra | Từ chối | Mã lỗi |
|---|---|---|
| V1 | Signer không thuộc policy đã ghim — kể cả người được thêm **sau khi** settlement được authorize | `NOT_A_SIGNER` |
| V2 | Chữ ký trên bất kỳ hash nào khác hash này | `BAD_SIGNATURE` |
| V3 | Cùng một signer ký lần hai để tự đạt ngưỡng | chặn bởi primary key |

V3 được thực thi bằng `PRIMARY KEY (settlement_id, signer_address)` chứ không bằng kiểm tra ở tầng
ứng dụng, vì ứng dụng có thể có bug còn index thì không.

## C4 · Weight và ngưỡng

Weight được cộng từ `settlement_approvals`, so với `required_weight` đã ghim trên settlement, và trả
về giao diện dưới dạng `collected / required` để màn hình approver hiện "1 trên 2".

Đạt ngưỡng thì phát một sự kiện chứ không submit ngay tại chỗ. Request HTTP của approver trả về
ngay khi chữ ký đã được lưu; việc submit là của một worker, và nó được phép chậm, được phép thất bại.

## C5 · Ghép và submit

Guard idempotency là `submit_attempted_at`, **không phải status**. Guard theo status trông như ngăn
được double submit; thực ra không: status là `releasing` suốt quá trình submit, nên một message
queue bị giao lại sẽ đi qua kiểm tra đó và submit lần thứ hai.

## C6 · Outcome chưa rõ — feature F-13

**`tx_bad_seq` và timeout là cùng một tình huống và được xử lý y hệt.** `tx_bad_seq` nói sequence
đã bị tiêu. Nó không nói ai tiêu, và nguyên nhân phổ biến nhất là *chính envelope này* đã lên
ledger — một lần submit trùng, một message bị giao lại, một phản hồi HTTP bị mất. Dựng transaction
mới với sequence mới rồi thu lại chữ ký nghĩa là chi cho vendor lần thứ hai.

Đường duy nhất được phép dẫn tới việc dựng lại:

1. Tra transaction của treasury **theo memo** — memo là request id, nên tra được mà không cần từng
   nhận được tx hash.
2. Nếu thấy, ghi hash và đi tiếp.
3. Nếu không thấy, submit lại **đúng envelope cũ** trước. Cùng hash, và cơ chế chống replay của
   mạng làm việc đó an toàn và rẻ.
4. Chỉ khi sequence chắc chắn bị một transaction **khác** tiêu mới được dựng lại — và điều đó nghĩa
   là làm phiền approver lần nữa.

## C7 · Fee-bump

Một transaction không được include vì phí thấp thì **không dựng lại**. Fee-bump của Stellar bọc một
envelope đã ký: chỉ fee source ký lớp bọc, và mọi chữ ký approver bên trong giữ nguyên hiệu lực.

Dựng lại vì lý do phí sẽ vứt bỏ những chữ ký hợp lệ và làm phiền hai người cho một vấn đề mà giao
thức đã tự giải quyết.

## C8 · Nhánh từ chối — feature F-12

Khi một approver từ chối:

1. Dừng hẳn việc submit. Không ghép envelope, không gửi gì cả.
2. **Giải phóng sequence đã giữ**, để treasury không bị chặn bởi một settlement sẽ không bao giờ đi
   tiếp.
3. Gọi `cancel_request` trên registry để trạng thái trên chain khớp.
4. Ghi một dòng `audit_events` nêu ai từ chối và khi nào.

Test nghiệm thu là một khẳng định phủ định trên danh sách transaction của treasury **trên chain** —
không phải trên database của ta. Database của ta ghi "cancelled" không chứng minh được gì về việc
tiền có chuyển hay không.

## Tổng hợp cách xử lý thất bại

| Kết quả | Xử lý | Không bao giờ |
|---|---|---|
| Thành công | Ghi hash, chuyển `releasing`, chờ readback | Ghi `paid` ở đây |
| `tx_bad_seq` | `unresolved` → reconcile theo memo | Dựng lại và thu lại chữ ký |
| Timeout | `unresolved` → reconcile theo memo | Giả định là thất bại |
| Phí không đủ | Fee-bump envelope đã ký | Dựng lại transaction |
| Thất bại dứt khoát | `failed`, ghi lại lý do | Tự động retry |
| Hết timebounds | Huỷ, dựng lại, thu lại chữ ký | Submit một transaction quá `maxTime` |

## Những quy tắc tài liệu này áp đặt

- **Khoá sequence theo tài khoản treasury.** Không phải theo lệnh chi, không phải theo tài sản.
- **Memo luôn là request id.** Đó là khoá tra ngược duy nhất sống sót khi mất tx hash.
- **Timebounds bắt buộc và hẹp.** Một giờ.
- **`tx_bad_seq` và timeout đi cùng một đường**, và đường đó đọc chain trước.
- **Guard idempotency là `submit_attempted_at`**, không bao giờ là status.
- **Từ chối thì giải phóng sequence.** Không thì một lần từ chối chặn treasury vô thời hạn.
- **Không bao giờ dựng lại transaction vì vấn đề phí.** Fee-bump nó.
