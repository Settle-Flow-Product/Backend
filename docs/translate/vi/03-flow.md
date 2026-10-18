# 03 · Luồng nghiệp vụ

> **Bản dịch tiếng Việt** của [`docs/03-flow.md`](../../03-flow.md).
> Bản tiếng Anh là bản có hiệu lực.

Ba mươi bước qua sáu lane, từ đăng nhập đến gói bằng chứng hoàn chỉnh, cùng máy trạng thái của một
lệnh chi.

Bản vẽ, dựng từ code đã ship chứ không phải từ đặc tả này, nằm ở
[`resources/flows/`](../../../resources/flows/).

## Sáu lane

| Lane | Ai | Loại node |
|---|---|---|
| Doanh nghiệp | Người dùng nội bộ tạo lệnh chi | user |
| Approver (2/3) | Những người ký | user |
| App tối thiểu | Bốn màn hình | interface |
| Backend | Mọi thứ ra quyết định | system · decision · error |
| Registry contract | Vòng đời trên chain | system · success · error |
| Stellar / treasury | Finality và việc chuyển tiền thật | system |

## Phần một — từ đăng nhập đến lệnh chi ở Pending

Mười ba bước. Điểm khó duy nhất là **S-10**: không bao giờ tin kết quả submit, chỉ tin trạng thái
đọc lại từ contract.

| Bước | Lane · loại | Node | Nội dung |
|---|---|---|---|
| S-01 | app · interface | Mở app SOW 1 | Công cụ nội bộ; chỉ người trong doanh nghiệp truy cập |
| S-02 | doanh nghiệp · user | Kết nối ví & ký challenge | Chứng minh quyền kiểm soát địa chỉ. Không chuyển tài sản |
| S-03 | backend · system | Xác minh chữ ký | Nonce một lần, gắn domain và network; chống replay |
| S-04 | app · interface | Mở form payment request | Bốn trường theo SOW: vendor, amount, asset, reference |
| S-05 | doanh nghiệp · user | Nhập dữ liệu lệnh chi | Địa chỉ nhà cung cấp, số tiền, loại tài sản, mã tham chiếu |
| S-06 | backend · system | Validate dữ liệu | `amount > 0`, asset được hỗ trợ, vendor là địa chỉ Stellar hợp lệ |
| S-07 | backend · decision | Dữ liệu hợp lệ? | Không tạo request khi còn field lỗi |
| S-08 | app · error | Báo lỗi đúng trường | Sửa tại chỗ rồi kiểm lại; không dùng toast chung chung |
| S-09 | contract · system | `create_request()` vào registry | Ghi lệnh chi lên chain. **Contract không giữ tiền** |
| S-10 | backend · decision | Readback: đã Pending? | Chỉ tin trạng thái đọc lại từ contract, không tin kết quả submit |
| S-11 | backend · error | Kết quả chưa rõ · readback | Đọc lại theo id deterministic; không tạo request thứ hai |
| S-12 | contract · success | Request ở trạng thái Pending | Đã có trên chain, chờ phê duyệt nội bộ |
| S-13 | backend · system | Thông báo cho approver | Gửi link màn hình duyệt kèm id lệnh chi |

### S-10 và S-11 là cặp node hay bị làm sai nhất

"Kết quả chưa rõ" không phải là lỗi để hiện toast rồi cho bấm lại. Đúng quy trình:

1. Đọc lại theo request id deterministic.
2. Nếu đọc thấy `Pending` thì coi như thành công.
3. Chỉ khi đọc không thấy sau vài lần backoff mới cho tạo mới, với một idempotency key **mới**.

Đường quay lại từ S-11 về S-09 là vòng **readback**, không phải vòng submit lại. Khác biệt này là
toàn bộ vấn đề: submit lại ghi ra một request thứ hai; đọc lại tìm thấy cái thứ nhất.

## Phần hai — từ phê duyệt 2/3 đến evidence hoàn chỉnh

Mười bảy bước. Đây là nơi chứa toàn bộ giá trị của SOW 1, và cũng là nơi một lỗi xử lý ngoại lệ dẫn
tới chi tiền hai lần.

| Bước | Lane · loại | Node | Nội dung |
|---|---|---|---|
| S-14 | app · interface | Approver mở màn hình duyệt | Hiện vendor, số tiền, tham chiếu và ai đã ký |
| S-15 | backend · system | `build_unsigned()` dựng tx | Payment tx từ treasury tới vendor, memo = request id |
| S-16 | backend · system | Khoá sequence của treasury | Một lệnh đang bay mỗi treasury; chống tranh chấp seq |
| S-17 | approver · user | Xem và quyết định | Kiểm tra vendor và số tiền trước khi ký |
| S-18 | backend · decision | Từ chối? | Nhánh reject phải dừng hẳn việc submit |
| S-19 | contract · error | Cancelled · không submit | Ghi trạng thái trên chain; khẳng định không có tx nào rời treasury |
| S-20 | approver · user | Ký tx hash trong ví | Chữ ký trên đúng hash đã dựng; đổi seq là chữ ký vô hiệu |
| S-21 | backend · system | Verify chữ ký & cộng weight | ed25519 trên tx hash; signer phải thuộc policy đã ghim |
| S-22 | backend · decision | Đủ threshold 2/3? | Cộng dồn weight của các signer đã ký |
| S-23 | app · interface | Chờ approver tiếp theo | Hiện tiến độ 1/2 và thời hạn timebounds của tx |
| S-24 | contract · system | `approve()` → Approved | Chỉ khi đủ weight mới chuyển trạng thái trên chain |
| S-25 | backend · system | Ghép chữ ký & submit | `DecoratedSignature` đúng hint, gửi lên Horizon |
| S-26 | Stellar · system | Stellar xử lý payment | Treasury chi tới địa chỉ vendor |
| S-27 | backend · decision | Outcome? | Confirmed, failed dứt khoát, hoặc chưa rõ |
| S-28 | backend · error | Reconcile trước khi retry | `tx_bad_seq` và timeout xử lý như nhau: đọc chain trước |
| S-29 | contract · success | `mark_paid()` → Paid | Registry ghi `Paid` kèm tx hash, sau readback |
| S-30 | app · success | Evidence đầy đủ | Tx hash từng mốc + link Stellar Expert cho reviewer |

### S-16 · Khoá sequence sớm, đừng để cuối

Chữ ký gắn chặt vào sequence của treasury. Hai lệnh chi song song lấy cùng seq thì lệnh thứ hai
chết bằng `tx_bad_seq` **sau khi** đã làm phiền hai người ký. Khoá phải nằm ở tầng DB từ Ngày 9.
Phát hiện ở Ngày 14 là mất một chặng.

### S-28 · Reconcile là đường duy nhất ra khỏi "chưa rõ"

`tx_bad_seq` nói sequence đã bị tiêu. Nó không nói ai tiêu, và nguyên nhân phổ biến nhất là chính
transaction đó đã lên ledger. Dựng transaction mới rồi thu lại chữ ký nghĩa là **chi hai lần**.

Xử lý y hệt timeout:

1. Tra transaction của treasury **theo memo** — memo chính là request id, nên tra được mà không cần
   biết tx hash.
2. Nếu thấy, ghi hash và đi tiếp; nó đã settle rồi.
3. Nếu không thấy, submit lại **đúng envelope cũ** trước. Cùng hash, và cơ chế chống replay của
   mạng làm việc đó an toàn.
4. Chỉ khi sequence chắc chắn đã bị một transaction **khác** tiêu mới được dựng lại — và điều đó
   nghĩa là làm phiền approver lần nữa.

## Máy trạng thái

```
                     create_request
                           |
                           v
                      +---------+   cancel_request    +-----------+
                      | Pending | ------------------> | Cancelled |   kết thúc
                      +---------+                     +-----------+
                           |                                ^
                           | approve()  (weight >= threshold)|
                           v                                 |
                      +----------+   cancel_request          |
                      | Approved | --------------------------+
                      +----------+
                           |
                           | mark_paid()  (sau readback từ ledger)
                           v
                       +------+
                       | Paid |   kết thúc
                       +------+


  Chỉ ở backend, không có trên chain:

       submitted --> unresolved --> (readback) --> confirmed | failed dứt khoát
                          ^                |
                          +----------------+
                            không bao giờ tự retry ra khỏi trạng thái này
```

**`Unresolved` chỉ tồn tại ở backend; trên chain không có trạng thái đó.** Nó là trạng thái hạng
nhất trong mọi máy trạng thái của hệ thống, không phải một trường hợp biên. Không gì ra khỏi nó
ngoài một readback.

Hai điều sơ đồ làm rõ:

- **`Cancelled` tới được từ cả `Pending` lẫn `Approved`.** Một lệnh chi đã approved nhưng chưa trả
  chính là thứ doanh nghiệp cần rút lại nhất.
- **Không có đường quay lại từ `Paid` hay `Cancelled`.** Cả hai là kết thúc trên chain. Sai sau
  điểm đó là một lệnh chi mới, không phải một lần sửa.

## Những quy tắc tài liệu này áp đặt

- **S-11 là vòng readback, không bao giờ là vòng submit lại.**
- **S-16 phải xong Ngày 9**, trước khi có thứ gì phụ thuộc vào nó.
- **S-19 được xác minh bằng khẳng định phủ định** trên danh sách transaction của treasury, không
  phải trên database của ta.
- **S-28 đọc chain trước mọi thứ khác**, không ngoại lệ, cho cả `tx_bad_seq` lẫn timeout.
- **S-29 đi sau một readback.** `mark_paid` không bao giờ được gọi dựa trên kết quả submit.
