# 02 · Module

> **Bản dịch tiếng Việt** của [`docs/02-modules.md`](../../02-modules.md).
> Bản tiếng Anh là bản có hiệu lực.

Danh sách công việc chính thức: 42 module trong 7 nhóm. Mỗi module có mã, thuộc deliverable nào, ai
làm, và điều kiện coi như xong. Bảng này để copy thẳng vào issue tracker.

| Nhóm | Số module | Deliverable |
|---|---|---|
| A · Smart contract — registry | 9 | 1 · 35% · $1.750 |
| B · Treasury và multisig | 5 | 2, phần một |
| C · Dịch vụ thu chữ ký | 8 | 2, phần hai |
| D · Đồng bộ chain, tối giản | 4 | hỗ trợ 2 và 3 |
| E · Giao diện tối thiểu | 5 | hỗ trợ cả ba |
| F · Dữ liệu | 6 | hỗ trợ 2 và 3 |
| G · Deploy và demo | 5 | 3 · 30% · $1.500 |

> **Deck ghi 37; nhưng chính các con số theo nhóm của nó cộng lại là 42.**
>
> A (9) + B (5) + C (8) + D (4) + E (5) + F (6) + G (5) = 42. Mọi con số theo nhóm ở đây khớp đúng
> với deck, nên tổng số là một lỗi cộng trong bản gốc chứ không phải thiếu một nhóm. Ghi lại ở đây
> để không ai "hoà giải" hai con số bằng cách lặng lẽ xoá năm dòng.

Nhóm **C** và **D** tập trung rủi ro kỹ thuật. Nhóm **G** tập trung rủi ro nghiệm thu: làm xong mọi
module khác mà đến buổi nghiệm thu không có tx hash thì vẫn bị đánh là thiếu bằng chứng.

## A · Smart contract — registry

*Deliverable 1 · 35% · $1.750 · Ngày 1–4*

| Mã | Module | Nội dung | Owner |
|---|---|---|---|
| **A1** | Storage layout & types | `DataKey` (`Admin`, `Settler`, `Req`); `PaymentRequest` gồm creator, payee, asset, amount, reference, expiry, status, `paid_tx`; `ReqStatus` 4 giá trị và 12 mã lỗi | Contract dev |
| **A2** | `create_request()` | Ghi lệnh chi mới. `require_auth` theo creator; kiểm `amount > 0` và expiry ở tương lai; chặn trùng id | Contract dev |
| **A3** | `approve()` | `Pending → Approved`. Chỉ settler, sau khi backend đã thu đủ weight ≥ threshold | Contract dev |
| **A4** | `mark_paid()` | `Approved → Paid` kèm tx hash. Chặn gọi hai lần, chặn khi quá hạn, kiểm amount khớp | Contract dev |
| **A5** | `cancel_request()` | Nhánh từ chối. `require_auth` theo creator; chỉ huỷ được khi chưa Paid | Contract dev |
| **A6** | `get_request()` · `is_payable()` | Hàm đọc, gọi qua RPC simulation. Không bao giờ submit như một transaction | Contract dev |
| **A7** | Events | `req_new`, `req_appr`, `req_paid`, `req_canc` — nguồn cho bộ đồng bộ chain | Contract dev |
| **A8** | Quản lý TTL | Bump TTL cả persistent lẫn instance storage ở mọi entry point. Quên instance là toàn bộ contract chết khi hết hạn | Contract dev |
| **A9** | Bộ test đơn vị | 16 ca: vòng đời, chống lạm dụng, biên. Coverage dòng ≥ 90% cho crate registry | Contract dev + Tester |

## B · Treasury và multisig

*Deliverable 2, phần một · Ngày 5–8*

| Mã | Module | Nội dung | Owner |
|---|---|---|---|
| **B1** | Script tạo treasury | Fund bằng friendbot, thiết lập trustline USDC testnet, chạy lại được từ đầu dưới 10 phút | Backend |
| **B2** | Cấu hình 3 signer | Ba signer weight 1, threshold low/med/high = 2, và `masterWeight = 0`. Thiếu bước cuối thì multisig chỉ là trang trí | Backend |
| **B3** | Guard chống khoá | Trước mọi `setOptions`: kiểm `Σ weight(signer còn hiệu lực) + masterWeight ≥ max(low, med, high)` trên **bộ giá trị mới** | Backend |
| **B4** | Policy & signer enrollment | Tạo policy version mới thay vì sửa tại chỗ. Settlement ghim `policy_version` lúc authorize | Backend |
| **B5** | Script xác minh cấu hình | Đọc lại account từ Horizon và assert đúng 3 signer, đúng threshold, master = 0. Output là evidence nộp kèm | Tester |

## C · Dịch vụ thu chữ ký

*Deliverable 2, phần hai · Ngày 9–17*

| Mã | Module | Nội dung | Owner |
|---|---|---|---|
| **C1** | `build_unsigned()` | Dựng payment tx treasury → vendor từ lệnh chi đã approved. Memo = request id, để tra ngược. Bắt buộc timebounds 1 giờ | Backend |
| **C2** | Khoá sequence number | Giữ seq ở tầng DB, unique index theo `(treasury_address, reserved_seq)`. Một lệnh đang bay mỗi treasury | Backend + Tech lead |
| **C3** | `submit_signature()` | Verify ed25519 trên đúng tx hash; signer phải thuộc policy đã ghim; primary key chặn ký trùng | Backend |
| **C4** | Cộng weight & phát hiện ngưỡng | Trả tiến độ `collected / required` cho giao diện; phát sự kiện khi đủ | Backend |
| **C5** | Ghép chữ ký & submit | `DecoratedSignature` đúng hint, gửi Horizon, ghi tx hash | Backend |
| **C6** | Xử lý outcome chưa rõ | `tx_bad_seq` và timeout xử lý như nhau: đọc chain theo memo trước, submit lại đúng envelope cũ, chỉ rebuild khi chắc chắn | Tech lead |
| **C7** | Fee-bump | Phí thấp thì bọc envelope đã ký — chữ ký approver giữ nguyên hiệu lực, không làm phiền lại | Backend |
| **C8** | Nhánh từ chối | Request bị reject thì dừng hẳn submit, giải phóng seq đã giữ, ghi audit | Backend |

## D · Đồng bộ chain — bản tối giản

*Hỗ trợ Deliverable 2 và 3 · Ngày 9–13*

| Mã | Module | Nội dung | Owner |
|---|---|---|---|
| **D1** | Ingest theo cursor | Đọc event từ Horizon/RPC theo cursor. Cursor chỉ tiến sau khi commit thành công | Tech lead |
| **D2** | Khử trùng tuyệt đối | `UNIQUE (tx_hash, op_index)` ở tầng DB. Hai bản daemon chạy song song vẫn an toàn | Tech lead |
| **D3** | Readback trước khi ghi Paid | Không bao giờ suy ra `paid` từ kết quả submit. Chỉ ghi sau khi đọc lại ledger | Tech lead |
| **D4** | Cập nhật read-model | Giao diện đọc từ read-model; không gọi Horizon đồng bộ trong đường request | Backend |

## E · Giao diện tối thiểu

*Đủ để demo — không phải dashboard*

| Mã | Module | Nội dung | Owner |
|---|---|---|---|
| **E1** | Đăng nhập bằng ví | Kết nối ví, ký nonce challenge, mở phiên làm việc nội bộ | Frontend |
| **E2** | Màn hình tạo lệnh chi | Bốn trường theo SOW, validate ngay tại field, xác nhận trước khi ghi lên chain | Frontend |
| **E3** | Danh sách lệnh chi | `Pending` / `Approved` / `Paid` / `Cancelled`, tiến độ chữ ký, lọc cơ bản | Frontend |
| **E4** | Màn hình approver ký | Hiện vendor, số tiền, tham chiếu, ai đã ký, và hạn của transaction. Kết nối ví và ký | Frontend |
| **E5** | Trang evidence | Bảng tx hash từng mốc kèm link Stellar Expert — **chính là** bằng chứng nộp cho Chapter Lead | Frontend |

## F · Dữ liệu — chỉ những bảng thật sự cần

| Mã | Bảng | Vì sao cần trong SOW 1 | Owner |
|---|---|---|---|
| **F1** | `payment_requests` | Bản sao off-chain để lọc và hiển thị nhanh. Chain vẫn là nguồn sự thật | Backend |
| **F2** | `settlements` | Có `request_id`, `treasury_address`, `reserved_seq`, `submit_attempted_at`. Thiếu bốn cột này là không nối được lệnh chi với transaction | Backend |
| **F3** | `settlement_approvals` | Chữ ký từng approver kèm weight. Khoá chính chặn ký trùng | Backend |
| **F4** | `chain_events` + `ingest_cursor` | Nhật ký ingest append-only và vị trí đọc. Xương sống của tính đúng đắn | Tech lead |
| **F5** | `idempotency_keys` | Chống tạo lệnh chi trùng khi mạng chập chờn | Backend |
| **F6** | `audit_events` | Ai làm gì, khi nào, từ trạng thái nào. Append-only, thu hồi quyền `UPDATE`/`DELETE` | Backend |

Hai bảng hỗ trợ không nằm trong danh sách của deck nhưng **B4** và **F-01** cần:
`governance_policies` + `policy_signers` (settlement không thể ghim một version không được lưu), và
`auth_challenges` (nonce không được lưu thì không thể dùng một lần). Cả hai được lập luận ở
[08-data-model.md](08-data-model.md).

## G · Deploy và demo

*Deliverable 3 · 30% · $1.500 · Ngày 18–20*

| Mã | Module | Nội dung | Owner |
|---|---|---|---|
| **G1** | Script deploy testnet | Deploy contract và treasury từ đầu, không thao tác tay bước nào. Chạy lại được nếu testnet reset | Contract dev |
| **G2** | Kịch bản demo round trip | Tạo lệnh chi → thu 2/3 chữ ký → chi tới vendor → registry đánh dấu paid | Tech lead |
| **G3** | Thu thập tx hash | Ghi hash ở từng mốc vào bảng evidence, kèm link Stellar Expert | Tester |
| **G4** | README + hướng dẫn deploy | Người ngoài đọc và tự deploy lại được từ đầu | Tech lead |
| **G5** | Video demo 3–5 phút | Cho Chapter Lead review không cần kiến thức kỹ thuật sâu | Tech lead |

## Module nằm ở đâu trong repository

| Nhóm | Nằm ở |
|---|---|
| A | `smart-contract/src/` |
| B | `backend/scripts/`, `backend/crates/settle-chain` |
| C | `backend/crates/settle-signature`, `settle-chain` |
| D | `backend/crates/settle-indexer`, `settle-backend/src/tasks.rs` |
| E | `frontend/website/` |
| F | `backend/migrations/`, `backend/crates/settle-db` |
| G | `smart-contract/scripts/`, `.deploy/evidence.md`, `README.md` |

## Những quy tắc tài liệu này áp đặt

- **Một module xong khi điều kiện hoàn thành của nó quan sát được**, không phải khi code compile.
- **C2 phải xong Ngày 9.** Đây là module duy nhất mà sự vắng mặt chỉ lộ ra sau khi đã lãng phí thời
  gian của hai approver.
- **C6 và C8 do Tech lead sở hữu**, vì cả hai là đường mà một sai lầm chi tiền hai lần.
- **G3 chạy trong buổi demo, không phải sau đó.** Hash không ghi ngay lúc đó là hash mất luôn.
