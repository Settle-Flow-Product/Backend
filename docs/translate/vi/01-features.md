# 01 · Feature

> **Bản dịch tiếng Việt** của [`docs/01-features.md`](../../01-features.md).
> Bản tiếng Anh là bản có hiệu lực.

Mười ba feature phải chạy được. Mỗi feature có mã, bên chịu trách nhiệm, và một cách nghiệm thu
quan sát được. **Không feature nào được nghiệm thu bằng lý do "đã code xong".**

Danh sách công việc thực hiện chúng nằm ở [02-modules.md](02-modules.md); luồng chúng ghép thành
nằm ở [03-flow.md](03-flow.md).

## Mười ba feature

| Mã | Feature | Mô tả | Nghiệm thu khi |
|---|---|---|---|
| **F-01** | Kết nối & xác thực ví | Người dùng nội bộ kết nối ví Stellar và ký một nonce challenge để chứng minh quyền kiểm soát địa chỉ. Không chuyển tài sản. | Đăng nhập được bằng ví; chữ ký sai bị từ chối |
| **F-02** | Tạo payment request | Nhập vendor address, amount, asset, reference. Ghi lên registry contract. | Tx hash của `create_request` trên Stellar Expert |
| **F-03** | Validate dữ liệu lệnh chi | `amount > 0` và tối đa 7 thập phân; asset được hỗ trợ; vendor là địa chỉ Stellar hợp lệ; reference 1–64 ký tự. | Mỗi ràng buộc có một unit test âm tính |
| **F-04** | Chống tạo trùng | Request id sinh deterministic từ idempotency key. Bấm hai lần hoặc retry sau timeout không tạo lệnh thứ hai. | Gọi API hai lần cùng key → cùng một request id |
| **F-05** | Theo dõi vòng đời trạng thái | `Pending → Approved → Paid`, cùng nhánh `Cancelled`. Trạng thái là nguồn sự thật trên chain. | Đọc `get_request` ở từng mốc thấy đúng trạng thái |
| **F-06** | Cấu hình treasury multisig | 3 signer weight 1, threshold low/med/high = 2, và `masterWeight = 0`. | Đọc account từ Horizon, khẳng định đúng cấu hình |
| **F-07** | Guard chống khoá tài khoản | Chặn mọi `setOptions` khiến tổng weight của signer nhỏ hơn threshold — tình huống không phục hồi được. | Test khẳng định cấu hình gây khoá bị từ chối |
| **F-08** | Dựng transaction chưa ký | Từ lệnh chi đã approved, dựng payment tx treasury → vendor, memo gắn request id, có timebounds. | XDR trả về decode được, memo khớp request id |
| **F-09** | Khoá sequence number | Một lệnh chi đang bay mỗi treasury. Lệnh thứ hai bị chặn sạch, không phải fail sau khi đã thu chữ ký. | Test đồng thời hai lệnh → lệnh thứ hai nhận `409` |
| **F-10** | Thu chữ ký approver | Verify ed25519 trên đúng tx hash; signer phải thuộc policy đã ghim; một signer chỉ ký một lần. | Chữ ký sai và signer lạ đều bị từ chối |
| **F-11** | Phát hiện đủ ngưỡng & submit | Cộng dồn weight; khi đạt 2 thì ghép `DecoratedSignature` và gửi lên Horizon. | Tx hash của lệnh chi, xem được trên Stellar Expert |
| **F-12** | Nhánh từ chối | Approver từ chối thì request chuyển `Cancelled` và dừng hẳn việc submit. | Test khẳng định không có tx nào rời treasury |
| **F-13** | Đánh dấu Paid sau readback | Chỉ gọi `mark_paid` sau khi đọc lại ledger. Timeout và `tx_bad_seq` đều đi qua reconcile. | Tx hash của `mark_paid`, và đọc lại thấy `Paid` |

## Ba chỗ dễ mất tiền nhất

Đây là những chỗ được code review kỹ nhất trong dự án.

**F-09 — tranh chấp sequence.** Chữ ký gắn chặt vào sequence number của treasury. Nếu hai lệnh chi
lấy cùng một sequence, lệnh thứ hai chết bằng `tx_bad_seq` *sau khi* đã làm phiền hai người ký.
Khoá phải nằm ở tầng DB từ Ngày 9, không phải phát hiện ở Ngày 14.

**F-12 — nhánh từ chối.** Nếu sai thì một lệnh chi đã bị từ chối vẫn rời khỏi treasury. Test là một
khẳng định phủ định: không có transaction nào rời tài khoản. Khẳng định phủ định rất dễ viết sai,
nên test này nêu đích danh tài khoản và kiểm danh sách transaction của nó.

**F-13 — xử lý `tx_bad_seq`.** Nếu phản ứng với `tx_bad_seq` là "dựng lại transaction rồi thu lại
chữ ký", hệ thống chi cho cùng một vendor hai lần. `tx_bad_seq` chỉ nói sequence đã bị tiêu; nó
không nói ai tiêu, và nguyên nhân phổ biến nhất là chính envelope đó đã lên ledger. Đọc chain trước.
Luôn luôn.

## Ánh xạ sang deliverable

| Deliverable | Feature |
|---|---|
| 1 — registry contract | F-02, F-03, F-04, F-05 |
| 2 — treasury và dịch vụ chữ ký | F-06, F-07, F-08, F-09, F-10, F-11, F-12 |
| 3 — deploy testnet và demo | F-13, và vòng round trip chạy qua tất cả những cái trên |
| Hỗ trợ | F-01 |

## Những quy tắc tài liệu này áp đặt

- **Không feature nào được nghiệm thu bằng "đã code xong".** Mọi dòng ở trên đều nêu một thứ quan
  sát được.
- **F-03 có một test âm tính cho mỗi ràng buộc.** Không phải một test chung cho "dữ liệu sai".
- **Test của F-12 là khẳng định phủ định về treasury**, không phải về database của ta.
- **F-09 và F-13 do Tech lead review**, không merge bằng một phê duyệt.
