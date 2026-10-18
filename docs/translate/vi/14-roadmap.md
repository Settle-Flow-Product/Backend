# 14 · Kế hoạch thực thi

> **Bản dịch tiếng Việt** của [`docs/14-roadmap.md`](../../14-roadmap.md).
> Bản tiếng Anh là bản có hiệu lực.

Hai mươi ngày làm việc trong năm chặng, bám đúng Daily Breakdown đã cam kết trong SOW. Mỗi chặng có
một đầu ra **đo được**, không phải một phần trăm hoàn thành.

## Ngân sách

| Deliverable | Tỷ trọng | Số tiền |
|---|---|---|
| 1 · Registry contract | 35% | $1.750 |
| 2 · Dịch vụ chữ ký | 35% | $1.750 |
| 3 · Deploy testnet và demo | 30% | $1.500 |
| | | **$5.000** |

## Vai trò

| Vai trò | Trọng tâm |
|---|---|
| Contract dev | Ngày 1–4, hỗ trợ Ngày 14–17 |
| Backend | Ngày 5–17, trọng tâm |
| Frontend | Ngày 5–20, song song |
| Tech lead | Xuyên suốt — khoá sequence, reconcile, nhánh từ chối, tài liệu |
| Tester | Script xác minh, integration, chaos, thu thập bằng chứng |

## Ngày 1–4 · Registry contract và unit test — nhóm A

| Việc | Chi tiết | Owner |
|---|---|---|
| Repo và CI | `fmt`, `clippy`, `test` chạy ở mỗi push. **Ghim Soroban SDK trong `Cargo.lock` ngay Ngày 1** | Tech lead |
| Storage và types | `DataKey`, `PaymentRequest`, `ReqStatus`, 12 mã lỗi. Chốt chiến lược TTL | Contract dev |
| `create_request` + `approve` + `mark_paid` + `cancel_request` | Đủ vòng đời `pending → approved → paid` và nhánh cancelled | Contract dev |
| 16 unit test | Vòng đời, chống lạm dụng, biên. Coverage ≥ 90% | Contract dev + Tester |

**Đầu ra:** contract compile được; unit test pass cho việc tạo request, phê duyệt và chuyển trạng
thái.

Grep gate no-custody là mục đầu tiên trong checklist review Ngày 4.

## Ngày 5–8 · Treasury multisig trên testnet — nhóm B

| Việc | Chi tiết | Owner |
|---|---|---|
| Treasury và trustline | Fund bằng friendbot, thiết lập trustline USDC testnet | Backend |
| 3 signer, threshold 2, `masterWeight` 0 | Bước `masterWeight` hay bị quên nhất — reviewer kiểm đầu tiên | Backend |
| Guard chống khoá tài khoản | Có test cho trường hợp cấu hình sẽ khoá vĩnh viễn | Backend |
| Script xác minh từ Horizon | In ra cấu hình thật để làm evidence | Tester |
| Song song: khung giao diện | Routing, design token, các component trạng thái | Frontend |

**Đầu ra:** treasury deploy trên testnet với cấu hình 3 signer được xác nhận và threshold được thực
thi.

## Ngày 9–13 · Dịch vụ thu chữ ký — nhóm C và D

| Việc | Chi tiết | Owner |
|---|---|---|
| `build_unsigned()` | Payment tx có `memo = request id` và timebounds 1 giờ | Backend |
| **Khoá sequence ở tầng DB** | Test đồng thời hai lệnh → lệnh thứ hai bị chặn sạch | Backend + Tech lead |
| `submit_signature()` | Verify ed25519 trên đúng tx hash; chặn ký trùng bằng khoá chính | Backend |
| Cộng weight, phát hiện đủ ngưỡng | Trả tiến độ `collected/required` cho giao diện | Backend |
| Bộ đồng bộ chain tối giản | Ingest theo cursor, khử trùng theo `(tx_hash, op_index)` | Tech lead |
| Màn hình tạo lệnh và danh sách | E2 và E3 | Frontend |

**Đầu ra:** dịch vụ nhận và lưu chữ ký của approver; phát hiện chính xác khi threshold được đáp ứng.

**Khoá sequence phải xong ở đây, Ngày 9.** Không phải Ngày 14. Đây là phần việc duy nhất mà sự vắng
mặt của nó chỉ lộ ra sau khi đã lãng phí thời gian của hai approver, và lúc đó buổi demo chỉ còn một
tuần.

## Ngày 14–17 · Submit, cập nhật registry, nhánh từ chối

| Việc | Chi tiết | Owner |
|---|---|---|
| Ghép chữ ký và submit | `tx_bad_seq` và timeout xử lý tách khỏi thất bại thật — cả hai đi qua reconcile | Tech lead |
| Nối registry: `approve` và `mark_paid` | Chỉ gọi sau readback xác nhận, không gọi sau submit | Backend + Contract dev |
| Nhánh từ chối | Test khẳng định không có transaction nào rời treasury | Backend + Tester |
| Integration trên testnet thật | Approve-then-settle và reject. Không mock | Tester |
| Chaos test | Giết service giữa lúc submit; khẳng định không double-submit sau khi khởi động lại | Tester + Tech lead |
| Màn hình ký duyệt | E4 | Frontend |

**Đầu ra:** integration test approve-and-settle pass đầy đủ; nhánh từ chối đánh dấu trạng thái đúng
và chặn việc submit.

## Ngày 18–20 · Demo round trip và tài liệu — nhóm G

| Việc | Chi tiết | Owner |
|---|---|---|
| Chạy full flow trên testnet | Tạo lệnh chi → 2/3 chữ ký → chi tới vendor → registry đánh dấu paid | Cả đội |
| Trang evidence và thu thập tx hash | Sáu mốc, đủ link Stellar Expert | Frontend + Tester |
| README và hướng dẫn deploy | Người ngoài tự deploy lại được từ đầu | Tech lead |
| Video demo 3–5 phút | Cho Chapter Lead review không cần kiến thức kỹ thuật sâu | Tech lead |
| Public repo và contract address | Dọn secret, thêm LICENSE, ghim contract ID vào README | Tech lead |

**Đầu ra:** một lệnh chi thu đủ 2/3 chữ ký, chi tới địa chỉ vendor, registry đánh dấu paid; tx hash
và hướng dẫn deploy đã commit.

## Đường găng

Ba phụ thuộc cứng, mỗi cái là một chỗ mà trượt một thứ sẽ trượt mọi thứ sau nó:

| Việc này | Không bắt đầu được trước | Vì |
|---|---|---|
| C1 `build_unsigned` | B2 xong | Chưa có treasury để lấy sequence number |
| C5 submit | C2 xong | Submit khi chưa khoá sequence chính là lỗi mà khoá sinh ra để ngăn |
| G2 demo | C6 và C8 xong | Một `tx_bad_seq` không xử lý trong buổi demo sẽ chi tiền hai lần trước mặt reviewer |

Nhóm E chạy song song từ Ngày 5 dựa trên hợp đồng API đã chốt
([09-api-contract.md](09-api-contract.md)) — đó là lý do tài liệu đó được viết trước khi cả hai bên
tồn tại.

## Những quy tắc tài liệu này áp đặt

- **SDK ghim ngay Ngày 1** và không đụng lại trong suốt chặng.
- **Khoá sequence xong Ngày 9.**
- **Chaos test chạy trước Ngày 18**, không phải trong buổi demo.
- **Trang evidence dựng trong Ngày 18–20**, và hash được thu ngay khi phát sinh — hash không ghi
  ngay lúc đó là hash mất luôn.
- **Một chặng xong khi đầu ra của nó quan sát được**, không phải khi các đầu việc được tích.
