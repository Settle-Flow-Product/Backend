# 12 · Kiểm thử

> **Bản dịch tiếng Việt** của [`docs/12-testing.md`](../../12-testing.md).
> Bản tiếng Anh là bản có hiệu lực.

Module A9 và phần integration ở Ngày 14–17. Mười sáu ca contract bắt buộc, coverage dòng ≥ 90% cho
crate registry, integration trên testnet thật không mock, và một chaos test.

## Tiêu chuẩn

**Khẳng định mã lỗi chính xác.** `assert_eq!(err, Error::AlreadyPaid)`, không bao giờ
`assert!(result.is_err())`. Một test chỉ kiểm *rằng* lời gọi thất bại sẽ pass khi contract thất bại
vì lý do sai — mà đó chính là loại bug đáng bắt. Đó là lý do mọi entry point trả `Result<_, Error>`.

**`require_auth` thất bại thì panic, không trả mã.** Đó là hành vi của Soroban host. Những ca đó
khẳng định panic, và tên test nói rõ điều đó để sau này không ai "sửa" chúng thành khẳng định mã.

**Tên test là câu tiếng Anh đầy đủ.** `a_request_cannot_be_paid_twice`, không phải
`test_mark_paid_2`. Tên là thứ reviewer đọc trong output của `cargo test`.

## Mười sáu ca bắt buộc

### Vòng đời — 5 ca

| # | Test | Khẳng định |
|---|---|---|
| 1 | `a_created_request_is_pending` | `create_request` → `get_request().status == Pending` |
| 2 | `the_settler_moves_a_pending_request_to_approved` | `create` → `approve` → `Approved` |
| 3 | `an_approved_request_becomes_paid_with_its_transaction_hash` | `approve` → `mark_paid` → `Paid`, và `paid_tx` được đặt |
| 4 | `the_creator_can_cancel_a_request_nobody_has_paid` | `create` → `cancel_request` → `Cancelled` |
| 5 | `an_expired_request_is_not_payable` | Đẩy ledger quá expiry → `is_payable() == false` |

### Chống lạm dụng — 7 ca

| # | Test | Khẳng định |
|---|---|---|
| 6 | `the_same_request_id_cannot_be_created_twice` | `RequestExists` — nửa on-chain của F-04 |
| 7 | `a_request_cannot_be_paid_twice` | `mark_paid` lần hai → `AlreadyPaid` |
| 8 | `only_the_settler_can_mark_a_request_paid` | `mark_paid` từ địa chỉ khác → **panic** |
| 9 | `only_the_settler_can_approve_a_request` | `approve` từ địa chỉ khác → **panic** |
| 10 | `a_request_cannot_be_approved_twice` | `approve` lần hai → `NotPending` |
| 11 | `only_the_creator_can_cancel_a_request` | `cancel_request` từ địa chỉ khác → **panic** |
| 12 | `paying_a_different_amount_is_refused` | `mark_paid` với amount lệch → `InvalidAmount` |

Ca 8, 9 và 11 là ma trận quyền. Thiếu chúng, một regression về phân quyền sẽ im lặng — contract vẫn
chạy đúng cho người gọi hợp lệ, và đó là đường duy nhất ai đó kiểm bằng tay.

### Biên — 4 ca

| # | Test | Khẳng định |
|---|---|---|
| 13 | `a_zero_or_negative_amount_is_refused` | Cả `0` lẫn `-1` → `InvalidAmount` |
| 14 | `an_expiry_in_the_past_is_refused` | `expiry <= ledger.timestamp()` → `InvalidExpiry` |
| 15 | `a_payment_landing_on_the_expiry_second_is_refused` | `mark_paid` đúng giây `expiry` → `Expired` |
| 16 | `an_entry_survives_across_ledgers_after_its_ttl_is_extended` | Đẩy nhiều ledger; cả request lẫn mọi hàm phụ thuộc instance storage vẫn chạy — module A8 |

Ca 15 là ranh giới quyết định phép so là `>=` hay `>`. Đặc tả nói một lệnh chi đến đúng giây hết hạn
là muộn, và test là thứ giữ điều đó qua mọi lần refactor.

Ca 16 phủ chế độ hỏng vô hình cho tới lúc thảm hoạ. Nếu instance TTL không được gia hạn, mọi hàm
trong contract ngừng chạy cùng một lúc, vài tuần sau khi deploy, và không test nào chạy trong một
ledger duy nhất nhận ra điều đó.

## Phủ mã lỗi

Mọi mã trong [04-contract-interface.md](04-contract-interface.md) đều có test chạm tới, **trừ**
`Unauthorized` — mã này cố tình không có test, vì hiện không có gì phát sinh nó: mọi kiểm tra quyền
đi qua `require_auth`, vốn panic trong host. Số này được dự trữ cho một kiểm tra mà host không diễn
đạt được.

Ghi lý do ra đây chính là điểm mấu chốt: một mã không được test mà không ai quyết định gì về nó là
một lỗ hổng; một mã không được test kèm lý do bằng văn bản là một quyết định.

## Test backend

| Test | Chứng minh | Feature |
|---|---|---|
| `two_concurrent_prepares_leave_one_reserved_sequence` | Hai `prepare` đồng thời: một `200`, một `409`, đúng một dòng `reserved_seq` còn lại | F-09 |
| `a_signature_over_a_different_hash_is_refused` | ed25519 verify trên đúng tx hash đã dựng | F-10 |
| `a_signer_outside_the_pinned_policy_is_refused` | Policy version ghim lúc authorize là cái chi phối | F-10 |
| `the_same_signer_cannot_sign_twice` | Primary key chặn, không phải code ứng dụng | F-10 |
| `the_same_idempotency_key_returns_the_same_request_id` | Gọi `POST /requests` hai lần tạo một lệnh chi | F-04 |
| `a_locking_threshold_configuration_is_refused` | Guard B3, đánh giá trên bộ giá trị mới | F-07 |
| `every_migration_reverses` | Mọi `up` rồi mọi `down`; không sót bảng nào và không sót kiểu enum nào | — |

## Integration — trên testnet thật, không mock

Ngày 14–17. Mock không có sequence number, không có trustline, và không bao giờ timeout.

| Kịch bản | Bước | Pass khi |
|---|---|---|
| **Approve và settle** | `create_request` → `prepare` → chữ ký 1 → chữ ký 2 → đủ ngưỡng → `approve` → submit → readback → `mark_paid` | `get_request` trả `Paid`, và mọi mốc đều có tx hash |
| **Từ chối** | `create_request` → `prepare` → approver từ chối | Request ở `Cancelled`, sequence đã giữ được giải phóng, **và danh sách transaction của treasury không có gì cho settlement này** |

Khẳng định của kịch bản từ chối là phủ định và được thực hiện trên **tài khoản treasury trên chain**,
không phải trên database của ta. Database ghi "cancelled" không chứng minh được gì về việc tiền có
chuyển hay không.

## Chaos test

Giết service giữa lúc submit. Khởi động lại. Khẳng định không có transaction thứ hai nào được gửi.

Đây là test chứng minh guard `submit_attempted_at` chứ không phải guard theo status. Với guard theo
status, service khởi động lại, thấy settlement ở `releasing`, và submit lần nữa — chi cho vendor hai
lần. Với guard theo lần thử, nó thấy `submit_attempted_at` đã đặt mà chưa có `tx_hash`, và đi vào
`reconcile_then_decide`, vốn đọc chain theo memo trước khi làm bất cứ điều gì.

Chạy nó **trước Ngày 18**, không phải trong buổi demo.

## Checklist trước demo

Mọi mục dưới đây đã từng làm hỏng một buổi demo ở đâu đó.

- [ ] Treasury có trustline USDC.
- [ ] Địa chỉ vendor có trustline USDC.
- [ ] Mọi ví test có trustline USDC.
- [ ] Treasury đủ XLM cho phí **và** minimum balance reserve.
- [ ] `masterWeight` đọc lại từ Horizon bằng 0.
- [ ] Script deploy đã chạy lại từ đầu, bấm giờ, và dưới 10 phút.
- [ ] Trang evidence hiện sáu dấu ✓, không còn dấu ○.

## Những quy tắc tài liệu này áp đặt

- **Khẳng định mã lỗi chính xác.** `is_err()` không phải một khẳng định.
- **Mười sáu ca là bắt buộc** bất kể con số coverage nói gì.
- **Lỗi quyền khẳng định panic**, và tên test nói rõ điều đó.
- **Integration chạy trên testnet, không mock.**
- **Test từ chối khẳng định trên treasury trên chain**, không phải trên database của ta.
- **Chaos test chạy trước Ngày 18.**
- **Một mã lỗi không được test phải kèm lý do bằng văn bản**, nếu không nó là một lỗ hổng.
