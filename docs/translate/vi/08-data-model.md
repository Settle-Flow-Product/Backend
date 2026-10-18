# 08 · Mô hình dữ liệu

> **Bản dịch tiếng Việt** của [`docs/08-data-model.md`](../../08-data-model.md).
> Bản tiếng Anh là bản có hiệu lực.
> **Toàn bộ DDL nằm ở bản tiếng Anh** — bản này giải thích lý do, không lặp lại mã.

PostgreSQL 17. Chỉ những bảng thật sự cần — module F1–F6, cộng hai bảng mà danh sách của deck hàm ý
nhưng không nêu tên.

## Bốn quyết định thiết kế

**1 · `numeric(30,7)`, không bao giờ float.** Stellar dùng bảy chữ số thập phân. `numeric` so sánh
bằng chính xác, điều kiện bắt buộc của một phép kiểm số tiền phải khớp. Một `double precision` ở bất
kỳ đâu trong đường tiền là một bug chờ đúng giá trị để nổ. Ở Rust cùng con số đó là `i128` stroops.

**2 · `request_id` là `bytea(32)`, không phải `uuid`.** Nó là `sha256(business_id ‖ idempotency_key)`
và **cũng chính là** khoá trên chain. Một giá trị cho cả hai loại bỏ toàn bộ một lớp bug ánh xạ id,
và làm cho retry-sau-timeout trở nên idempotent ở tầng contract.

**3 · Policy được đánh version, không sửa.** Đổi signer hay threshold thì ghi dòng mới với
`version + 1`. Settlement ghim version nó được authorize theo, nên nhiều tháng sau câu hỏi "lệnh này
được duyệt theo luật nào?" vẫn có câu trả lời.

**4 · Hai bảng append-only, thực thi bằng quyền.** `chain_events` và `audit_events` không bao giờ
được UPDATE hay DELETE, và vai trò ứng dụng **không có quyền** để thử.

## Các bảng

| Migration | Bảng | Module | Vì sao tồn tại |
|---|---|---|---|
| 0001 | *(kiểu và domain)* | — | Một chỗ định nghĩa mọi enum và kiểu tiền |
| 0002 | `auth_challenges` | F-01 | Nonce không được lưu thì không thể dùng một lần |
| 0003 | `governance_policies`, `policy_signers` | B4 | Settlement không thể ghim một version không được lưu |
| 0004 | `payment_requests`, `idempotency_keys` | F1, F5 | Lọc và hiển thị nhanh; chống tạo trùng |
| 0005 | `settlements` | F2 | Nối lệnh chi với transaction, và giữ sequence đã đặt chỗ |
| 0006 | `settlement_approvals` | F3 | Chữ ký và weight của từng approver |
| 0007 | `chain_events`, `ingest_cursor` | F4 | Nhật ký ingest và vị trí đọc |
| 0008 | `audit_events` | F6 | Ai làm gì, khi nào, từ trạng thái nào |

File migration đặt tên theo câu văn mô tả thay đổi, cả hai chiều:
`0005_a_settlement_reserves_one_sequence.up.sql` và `.down.sql`. CI áp dụng mọi `up` theo thứ tự,
rồi mọi `down` theo chiều ngược, và fail nếu còn sót bảng **hoặc kiểu enum** nào.

## Vài chi tiết đáng giải thích

**`auth_challenges`** — xác minh **lấy** dòng đó: đọc và xoá trong một câu lệnh, nên một challenge
bị bắt được không thể replay kể cả với một client chậm. `SELECT` rồi `DELETE` có một khe hở giữa hai
lệnh; `DELETE … RETURNING` thì không.

**Trigger trên `governance_policies`** — khi activate, nó khẳng định tổng weight của signer đạt ít
nhất threshold. Một policy mà signer không đạt nổi threshold của chính nó là một cái bẫy: doanh
nghiệp không bao giờ chi được cho ai, và phát hiện ra vào lúc tệ nhất.

**`payment_requests.reference` và `reference_hash`** — chuỗi thô nằm **ở đây**. Chỉ hash lên chain:
một mã đơn hàng ghi lên ledger là công khai vĩnh viễn và gắn vĩnh viễn với một địa chỉ vendor và một
số tiền, tức là công bố quan hệ nhà cung cấp của doanh nghiệp cho bất kỳ ai muốn xem.

**`settlements.submit_attempted_at`** — tách khỏi `status` vì guard theo status **không** idempotent:
status là `releasing` suốt quá trình submit, nên một message bị giao lại sẽ qua kiểm tra status và
submit lần hai.

## Hai index mang tính đúng đắn

| Index | Ngăn chặn |
|---|---|
| `settlements_one_in_flight_per_treasury ON settlements (treasury_address)` | Hai lệnh chi tranh chấp một sequence number — F-09 |
| `UNIQUE (tx_hash, op_index)` trên `chain_events` | Một event bị xử lý hai lần, và làm ingest song song an toàn — D2 |

Cả hai thay thế rất nhiều logic phòng thủ ở tầng ứng dụng, và — quan trọng hơn — cả hai vẫn đúng khi
hai bản của một dịch vụ chạy cùng lúc, điều mà logic ứng dụng không làm được.

`settlements_one_in_flight_per_treasury` là **dòng quan trọng nhất trong schema này**. Khoá nó theo
bất cứ thứ gì hẹp hơn địa chỉ treasury sẽ tái tạo đúng lỗi mà nó sinh ra để ngăn.

## Những quy tắc tài liệu này áp đặt

- **Tiền là `numeric(30,7)`.** Không bao giờ `float`, không bao giờ `double precision`.
- **`request_id` là một giá trị dùng chung với chain**, sinh deterministic, không ngẫu nhiên.
- **Policy được đánh version và ghim**, không sửa tại chỗ.
- **`settlements_one_in_flight_per_treasury` khoá theo địa chỉ treasury.** Không gì hẹp hơn.
- **`submit_attempted_at` là guard idempotency**, không phải `status`.
- **Bảng append-only thực thi bằng `REVOKE`**, không bằng quy ước.
- **Mọi migration đảo ngược được**, và CI chứng minh điều đó ở mỗi lần push.
