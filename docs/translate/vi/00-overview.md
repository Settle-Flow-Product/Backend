# 00 · Tổng quan

> **Bản dịch tiếng Việt** của [`docs/00-overview.md`](../../00-overview.md).
> Bản tiếng Anh là bản có hiệu lực.

SOW 1 xây gì, dứt khoát không xây gì, và Instaward ràng buộc những gì.

Tài liệu này không mô tả cách hoạt động. Feature nằm ở [01-features.md](01-features.md), danh sách
công việc ở [02-modules.md](02-modules.md), giao diện contract ở
[04-contract-interface.md](04-contract-interface.md).

## Vấn đề

Doanh nghiệp giữ stablecoin treasury trên Stellar không có cách nào được thiết kế riêng để chi tiền
cho nhà cung cấp với quyền kiểm soát phê duyệt nội bộ.

Zebec đã phủ payroll trên Stellar. COINQVEST đã phủ merchant checkout. Không ai xử lý accounts
payable — chi cho nhà cung cấp với một bước duyệt ngân sách trước khi tiền rời tài khoản. Đội ngũ
còn lại hai lựa chọn tồi: chi thẳng từ ví không có dấu vết phê duyệt, hoặc quay về công cụ hướng
ngân hàng như Bill.com vốn không chạm được stablecoin.

Tài khoản Stellar đã hỗ trợ sẵn multi-signature với weighted signer và threshold. Đây là nguyên
thủy đã chạy production nhiều năm. Chưa ai nối nó vào một quy trình lệnh chi thực tế. SOW 1 xây
đúng lớp còn thiếu đó, và cố tình không chạm custody — đó là điều khiến 20 ngày không cần audit trở
thành một lịch trình trung thực chứ không phải một hy vọng.

## Mục tiêu

Kết thúc 20 ngày làm việc: contract Payment Request Registry đã deploy trên Stellar testnet, cùng
một tài khoản treasury cấu hình 3 signer có trọng số và threshold 2. Một lệnh chi tạo trên registry
có thể được 2 trong 3 signer phê duyệt và chi tới địa chỉ vendor, với registry đánh dấu paid sau khi
transaction settle.

## Ba deliverable

| | Tỷ trọng | Nội dung |
|---|---|---|
| **1** | 35% · $1.750 | Contract Soroban tạo lệnh chi gồm địa chỉ vendor, số tiền, tài sản và mã tham chiếu, theo dõi qua `pending → approved → paid`. Contract không bao giờ giữ tiền — nó ghi nhận và xác minh trạng thái. |
| **2** | 35% · $1.750 | Dịch vụ backend dựng transaction Stellar chưa ký từ lệnh chi đã approved, nhận chữ ký của từng approver, và submit khi tổng weight đạt ngưỡng của treasury. |
| **3** | 30% · $1.500 | Registry và treasury 3 signer deploy trên testnet. Demo một lệnh chi thu đủ 2/3 chữ ký, chi tới địa chỉ vendor, registry đánh dấu paid — kèm tx hash tra cứu được. |

## Phạm vi

Rủi ro lớn nhất của một Instaward 20 ngày là mở rộng phạm vi. Cột bên phải là ranh giới cứng: một
pull request chạm vào đó bị đóng, kể cả khi nó "chỉ mất một buổi".

### Trong phạm vi

- Contract sổ đăng ký lệnh chi, không custody.
- Vòng đời `pending → approved → paid`, cùng nhánh `cancelled`.
- Treasury G-account: 3 signer weight 1, threshold 2.
- Dựng transaction chưa ký từ một lệnh chi đã approved.
- Thu chữ ký từng approver, kiểm ngưỡng, submit.
- Đọc lại chain để xác nhận trước khi ghi `paid`.
- Giao diện tối thiểu — đủ để chứng minh deliverable.
- Deploy testnet, demo round trip, và gói bằng chứng.

### Ngoài phạm vi — trích nguyên văn SOW

| Không xây | |
|---|---|
| Dashboard đầy đủ | Chỉ những màn hình tối thiểu buổi demo cần |
| Vendor-facing views | Không có cổng cho nhà cung cấp |
| Reconciliation indexer | Không xây dịch vụ đối soát riêng |
| Accounting export | Không xuất dữ liệu kế toán |
| Security audit | Không thuê kiểm định trong 30 ngày này |
| Mainnet deployment | Chỉ testnet |

Thêm bốn thứ nữa, đặt ra để bảo vệ lịch chứ không phải vì SOW nêu tên: **không escrow giữ tiền,
không off-ramp fiat, không đa ngôn ngữ, không ứng dụng di động.**

## Ràng buộc Instawards

| | Ràng buộc | Nghĩa là |
|---|---|---|
| 01 | ≤ 30 ngày | Toàn bộ phạm vi hoàn thành trong 30 ngày lịch, khoảng 20 ngày làm việc |
| 02 | Thực thi, không thăm dò | Instawards tài trợ thứ chạy được, không tài trợ nghiên cứu mở |
| 03 | Tối đa 2 lần tiếp nối | Mỗi dự án nhận không quá hai Instaward tiếp theo |
| 04 | Trần $5.000 / $15.000 | Mỗi Instaward tối đa $5.000; tổng cộng dồn không quá $15.000 |

## Một điều cần thống nhất trước Ngày 1

Văn bản SOW mô tả chi tiền cho nhà cung cấp — accounts payable. Sản phẩm thương mại SettleFlow phát
triển theo chiều ngược lại, thu tiền từ khách hàng. **Về kỹ thuật, sổ đăng ký và cơ chế phê duyệt là
cùng một thứ cho cả hai chiều**, nên code giao cho Instaward vẫn đúng nguyên vẹn.

Tài liệu này bám đúng văn bản SOW. Mọi thứ dưới đây mô tả luồng chi tiền cho vendor.

## Tài sản

| | USDC | XLM |
|---|---|---|
| Dạng | Stellar Asset Contract | Native |
| Số thập phân | 7 | 7 |
| Dung sai số tiền | 0 — phải đúng | 0 — phải đúng |
| Trustline bắt buộc ở địa chỉ nhận | Có | Không |
| Minimum balance reserve | — | 1 XLM nền, cộng 0,5 mỗi subentry |
| Phí giao dịch trả bằng | XLM | XLM |
| Độ sâu xác nhận | 1 ledger, khoảng 5 giây | 1 ledger, khoảng 5 giây |
| Rủi ro reorg | Không — SCP không fork | Không |

Hai hệ quả đã gây sự cố thật ở những hệ thống dạng này:

- **Phí luôn bằng XLM, kể cả khi chi USDC.** Một treasury giữ một triệu USDC và 0,9 XLM sẽ qua mọi
  kiểm tra số dư rồi fail ở ledger — sau khi hai approver đã ký. Kiểm tài sản và kiểm phí là hai
  gate độc lập.
- **Ví có 10 XLM không chi được 10 XLM.** Minimum balance reserve không tiêu được.

Readback vẫn bắt buộc dù không có rủi ro reorg. Lý do là hạ tầng — mất một phản hồi HTTP, đứt kết
nối — chứ không phải đồng thuận.

## Mô hình tin cậy

| Bên | Được tin về | Không được tin về |
|---|---|---|
| Doanh nghiệp | Tạo và huỷ lệnh chi của chính mình | Đánh dấu lệnh chi của mình là paid — chỉ settler làm được |
| Approver | Đóng góp weight chữ ký | Mọi thứ khác; một chữ ký đáng đúng bằng weight của nó |
| Backend của ta | Dựng transaction, thu chữ ký, đọc chain | Là nguồn sự thật cho trạng thái của một lệnh chi |
| Ví | Giữ khoá và ký | Báo kết quả; phản hồi lạc quan không phải là xác nhận |
| Stellar | Finality, và định nghĩa "bây giờ" cho hết hạn | Luôn liên lạc được — mọi lời gọi cần một nhánh chưa rõ |

## Những quy tắc bản build này tuân theo

- **Registry không bao giờ chuyển token.** Kiểm bằng grep gate trong CI, không phải bằng trí nhớ.
  Đây là mục đầu tiên trong checklist review Ngày 4.
- **Chain là nguồn sự thật.** `paid` chỉ được ghi sau readback. Submit thành công không phải là
  lệnh chi đã trả.
- **Chưa rõ là một trạng thái, không phải một lỗi.** `tx_bad_seq` và timeout xử lý y hệt nhau: đọc
  chain trước, submit lại đúng envelope cũ, chỉ dựng lại khi chắc chắn chưa có gì lên chain.
- **Thời gian ledger quyết định hết hạn.** `env.ledger().timestamp()`, không bao giờ dùng đồng hồ
  server.
- **Tiền không bao giờ là float.** `numeric(30,7)` ở Postgres, `i128` stroops ở Rust, `String` trên
  wire — số trong JSON là IEEE double.
- **Mọi endpoint chạm chain trả 202**, không phải 200, để không client nào hiểu "API OK" thành
  "tiền đã chuyển".
- **Màu không bao giờ là kênh thông tin duy nhất.** Mọi pill trạng thái đều kèm chữ.
