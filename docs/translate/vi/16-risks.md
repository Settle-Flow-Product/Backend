# 16 · Rủi ro và câu hỏi còn mở

> **Bản dịch tiếng Việt** của [`docs/16-risks.md`](../../16-risks.md).
> Bản tiếng Anh là bản có hiệu lực.

Sáu rủi ro có thể làm trượt 20 ngày, mỗi cái kèm một phương án là **một phần việc** chứ không phải
một ý định. Ba câu hỏi phải chốt trước Ngày 1.

## Sáu rủi ro

| Rủi ro | Khả năng | Ảnh hưởng | Cách giảm |
|---|---|---|---|
| **Sequence number tranh chấp** | Cao | Chữ ký đã thu bị vô hiệu, phải làm lại, mất niềm tin ngay tại buổi demo | Khoá seq ở tầng DB **Ngày 9**, có test đồng thời. Đừng để tới Ngày 14 mới phát hiện |
| **Mở rộng phạm vi** | Cao | Không kịp giao ba deliverable đã cam kết | Danh sách ngoài phạm vi ở [00-overview.md](00-overview.md) là ranh giới cứng. PR chạm vào đó bị đóng ngay |
| **Quên `masterWeight = 0`** | Trung bình | Multisig thành hình thức — người giữ khoá treasury ký một mình vẫn được. Reviewer sẽ phát hiện | Đưa vào script B2, và script B5 assert lại. Không bao giờ làm bằng tay |
| **Testnet reset hoặc chập chờn** | Trung bình | Mất contract đã deploy và dữ liệu demo | Script deploy dựng lại toàn bộ từ đầu dưới mười phút, không thao tác tay bước nào |
| **Soroban SDK đổi phiên bản** | Trung bình | Contract không build, mất 1–2 ngày | Ghim phiên bản trong `Cargo.lock` ngay Ngày 1. Không update giữa chặng |
| **Trustline USDC testnet** | Trung bình | Payment fail ngay tại buổi demo, trước mặt reviewer | Checklist trước demo: treasury, địa chỉ vendor và mọi ví test đều có trustline |

Hai trong số này có cùng một hình dạng và đáng chú ý cùng nhau. **Tranh chấp sequence** và
**`masterWeight`** đều hỏng một cách im lặng dưới điều kiện dùng bình thường — một luồng, một người
vận hành — và chỉ lộ ra dưới điều kiện của một buổi demo: đồng thời và bị soi. Không cái nào bị bắt
bởi việc thử happy path bằng tay, đó là lý do cả hai đều có một kiểm tra tự động cụ thể gắn kèm chứ
không phải một lời nhắc.

## Ba câu hỏi phải chốt trước Ngày 1

### 01 · Settler là ai?

Địa chỉ nào được gọi `approve` và `mark_paid` — một khoá backend riêng, hay chính tài khoản treasury?

Điều này định hình trực tiếp thiết kế quyền của contract. Một khoá backend riêng nghĩa là registry
tin một dịch vụ, và chiếm được dịch vụ đó cho phép ai đó đánh dấu paid những lệnh chưa từng được
trả — tệ, nhưng **không có tiền nào chuyển**, vì registry không giữ đồng nào. Dùng chính tài khoản
treasury nghĩa là đúng hai chữ ký đã cho phép chi tiền cũng cho phép đổi trạng thái, chặt chẽ hơn
nhưng đưa `approve` và `mark_paid` vào sau multisig, tức là thêm một vòng ký mỗi hàm.

Thiết kế ở [04-contract-interface.md](04-contract-interface.md) giả định một khoá backend riêng. Nếu
đổi, `approve` và `mark_paid` trở thành thao tác đa chữ ký và luồng ở [03-flow.md](03-flow.md) có
thêm hai vòng ký.

### 02 · Contract có nâng cấp được không?

Nếu deploy là bất biến, một lỗi phát hiện sau Ngày 20 nghĩa là deploy một contract mới và chuyển mọi
tham chiếu sang nó. Nếu nâng cấp được thì có người giữ quyền đó, và người đó trở thành điểm tin cậy
đơn lẻ lớn nhất của hệ thống.

Với một registry không giữ tiền, bất biến là lựa chọn bảo vệ được: kết cục xấu nhất là một registry
lỗi thời bị thay thế. Cùng câu hỏi đó cho một contract giữ tiền sẽ có câu trả lời khác — đó là một
phần lý do custody nằm ngoài phạm vi.

### 03 · Reference lưu hash hay chuỗi thô?

Đặc tả này lưu **hash** lên chain và giữ chuỗi thô ở Postgres.

Reference là mã đơn hàng hoặc số hoá đơn. Ghi lên ledger dưới dạng chuỗi thô thì nó công khai vĩnh
viễn và gắn vĩnh viễn với một địa chỉ vendor và một số tiền — tức là quan hệ nhà cung cấp và quy mô
chi trả của doanh nghiệp trở nên ai cũng đọc được. Hash đủ để chứng minh tương ứng khi đối soát mà
không công bố quan hệ thương mại.

Nếu quyết định đi theo hướng khác, `PaymentRequest.reference` đổi kiểu và
[08-data-model.md](08-data-model.md) mất cột `reference_hash`.

## Sau Instaward này

SOW mục 7.1 dự kiến xin một Instaward tiếp nối. Registry và cơ chế phê duyệt xây trong 20 ngày này
chính là nền mà dashboard, đối soát và cổng nhà cung cấp dựng trực tiếp lên. Trong lộ trình 6 tháng
của SettleFlow, đây là tháng 1.

Ba câu hỏi mà sản phẩm đầy đủ phải trả lời trước khi mở bất kỳ phạm vi custody nào — không câu nào
là kỹ thuật:

1. **Nâng cấp.** Ai được nâng cấp một contract giữ tiền, và nếu không ai được thì vá lỗi bằng cách
   nào?
2. **Thoát hiểm.** Nếu contract bị khoá, doanh nghiệp lấy lại tiền bằng cách nào — và cơ chế đó có
   tự nó là lỗ hổng không?
3. **Pháp lý.** Ai chịu trách nhiệm cho số tiền contract đang giữ, và điều đó thay đổi nghĩa vụ
   KYB/AML ở từng thị trường như thế nào?

## SOW 1 trên một trang

| Xây gì | Không xây gì | Nghiệm thu bằng |
|---|---|---|
| Contract sổ đăng ký lệnh chi, không custody | Dashboard đầy đủ | Repo công khai |
| Treasury 3 signer, cần 2 chữ ký | Cổng cho nhà cung cấp | Contract ID trên testnet |
| Dịch vụ dựng tx, thu chữ ký, submit | Indexer đối soát | Bảy transaction hash |
| Bốn màn hình đủ demo | Xuất dữ liệu kế toán | Video demo 3–5 phút |
| Deploy testnet và bằng chứng | Kiểm định bảo mật | Hướng dẫn deploy chạy lại được |
| | Mainnet | |

| 13 | 42 | 30 | 4 | 7 | 6 |
|---|---|---|---|---|---|
| feature | module trong 7 nhóm | bước nghiệp vụ | màn hình | mốc bằng chứng | rủi ro đã có phương án |

## Câu chốt

Stellar đã có multisig gốc chạy production nhiều năm, nhưng chưa ai nối nó vào một quy trình duyệt
chi thật. SOW 1 xây đúng lớp đó trong 20 ngày, không chạm vào custody, và giao bằng chứng mà bất kỳ
ai cũng kiểm chứng được trên blockchain công khai.
