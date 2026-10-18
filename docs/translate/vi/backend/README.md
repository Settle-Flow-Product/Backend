# Backend chạy thế nào

> **Bản dịch tiếng Việt** của [`docs/backend/README.md`](../../../backend/README.md).
> Bản tiếng Anh là bản có hiệu lực.

Một binary, bảy crate, một database. File này giải thích chuyện gì xảy ra khi khởi động và chuyện gì
xảy ra khi ai đó tạo một lệnh chi — theo **đúng thứ tự code thực thi**, không theo thứ tự các module
được liệt kê.

## Hình dạng

```
                          settle-backend                 binary
                   http · config · state · tasks
                     registry · horizon · main
                                |
        +-------------+---------+---------+-------------+
        |             |                   |             |
  settle-signature  settle-auth    settle-indexer       |
        |             |                   |             |
        +------+------+---------+---------+-------------+
               |                |
          settle-db       settle-chain
               |                |
               +-------+--------+
                       |
                 settle-domain          không phụ thuộc gì
```

Đọc từ dưới lên. `settle-domain` không phụ thuộc gì cả — nó là hàm thuần trên kiểu dữ liệu đơn giản,
đó là thứ cho phép test các quy tắc trong vài micro-giây. Không tầng nào phía trên được cài đặt lại
một quy tắc đã sống ở đó.

## Mỗi crate làm gì

| Crate | Giữ |
|---|---|
| `settle-domain` | Quy tắc validate, các enum trạng thái, kiểu stroops, cách sinh `request_id`, cấu trúc lỗi. Không I/O, không async, không database |
| `settle-chain` | Mọi thứ Stellar: dựng transaction, tính hash, verify ed25519, gắn decorated signature, decode event |
| `settle-db` | Mọi câu SQL trong hệ thống, một module cho mỗi mối quan tâm. Dùng `sqlx::query` runtime, không dùng macro |
| `settle-auth` | Challenge của ví, xác minh, và session token |
| `settle-signature` | Deliverable 2: dựng tx, giữ sequence, nhận chữ ký, submit, reconcile |
| `settle-indexer` | Một phép fold thuần từ luồng event sang projection, cùng quy tắc cursor |
| `settle-backend` | Server axum, cấu hình, state dùng chung, và các vòng lặp nền |

## Khởi động

`main` làm sáu việc, theo thứ tự mang tính quyết định:

```
1  kết nối Postgres và chạy migration       thất bại ở đây, không phải sau đó
2  kiểm registry trả lời, treasury tồn tại
3  dựng chain client
4  spawn indexer và submitter
5  bind cổng
6  serve, có graceful shutdown
```

Bước 2 đứng trước bước 5 có chủ đích. Một backend bind cổng rồi mới phát hiện không với tới chain sẽ
tự báo là khoẻ trong khi vô dụng, và load balancer sẽ tin nó.

## Tạo một lệnh chi, từng bước

1. **`POST /requests` tới `http.rs`.** Header `Idempotency-Key` là bắt buộc; thiếu nó thì bị từ chối
   trước mọi thứ khác.
2. **`settle-domain` validate.** Địa chỉ vendor theo `^G[A-Z2-7]{55}$`, số tiền dạng chuỗi thập phân
   tối đa bảy chữ số, tài sản trong tập được hỗ trợ, độ dài reference. Mỗi lỗi nêu tên trường của
   nó.
3. **`settle-db` claim idempotency key.** Việc này xảy ra **trước** khi ghi bất cứ thứ gì. Bốn kết
   quả: claim được và đi tiếp; đã xong nên trả lại kết quả cũ; đang bay nên trả `409`; cùng key khác
   payload nên trả `IDEMPOTENCY_CONFLICT`.
4. **Sinh request id.** `sha256(business_id ‖ idempotency_key)`, 32 byte. Đây là thứ khiến một lần
   retry sau timeout rơi vào đúng lệnh chi cũ thay vì tạo cái thứ hai.
5. **`settle-chain` gọi `create_request`.**
6. **Chưa tin gì cả.** Kết quả submit không phải câu trả lời. Đọc lại request từ contract theo id.
   Nếu đọc thấy `Pending` thì thành công. Nếu submit timeout và đọc lại không thấy, kết quả là
   *chưa rõ* — không phải thất bại, và không phải lời mời retry mù.
7. **Trả `202 Accepted`**, với `status: "pending"` và tx hash rỗng. Giao diện hiện "đang ghi lên
   chain" và poll. Nó không hiện thành công, vì ngay lúc đó không ai biết.

## Rồi ai đó phê duyệt

1. **`POST /requests/:id/prepare`.** Đọc tài khoản treasury từ Horizon — lời gọi mạng duy nhất không
   tránh được trong một đường người dùng đang chờ, vì sequence number chỉ tồn tại trên mạng.
2. **Giữ sequence ở database.** Unique index trên `treasury_address` khiến `prepare` thứ hai nhận
   `409` ngay. Đây là dòng quan trọng nhất trong schema.
3. **Dựng transaction** — payment tới vendor, `memo = request id`, timebounds một giờ. Hash của nó
   là thứ approver ký, và nó đổi nếu sequence đổi.
4. **`POST /requests/:id/signatures` tới hai lần.** Mỗi lần ba kiểm tra: signer thuộc **policy version
   đã ghim**; chữ ký verify trên đúng hash này; và khoá chính `(settlement_id, signer_address)` từ
   chối chữ ký thứ hai của cùng một người.
5. **Đạt weight 2 thì phát sự kiện** và request HTTP trả về. Việc submit là của một worker.

## Các vòng lặp nền

**Indexer** đọc event theo cursor, chèn vào `chain_events`, và tiến cursor **sau** khi commit. Chết
giữa hai bước thì đọc lại, và đọc lại vô hại nhờ `UNIQUE (tx_hash, op_index)`.

**Submitter** nhặt các settlement đã đạt ngưỡng. Nó kiểm `submit_attempted_at` — **không phải**
`status`, vốn là `releasing` suốt quá trình submit và do đó không ngăn được một message bị giao lại.

| Kết quả | Xảy ra gì |
|---|---|
| Thành công | Ghi hash, chuyển `releasing`, chờ indexer thấy nó settle |
| `tx_bad_seq` | **Chưa rõ.** Không phải thất bại, không phải dựng lại |
| Timeout | **Chưa rõ.** Xử lý y hệt `tx_bad_seq` |
| Phí không đủ | Fee-bump envelope đã ký. Chữ ký approver giữ nguyên hiệu lực |

`tx_bad_seq` nói sequence đã bị tiêu. Nó không nói ai tiêu, và nguyên nhân phổ biến nhất là chính
envelope đó đã lên ledger. Cả hai ca chưa rõ đều đi vào `reconcile_then_decide`, vốn tra transaction
của treasury **theo memo** và chỉ dựng lại khi sequence chắc chắn bị thứ khác tiêu.

## Ghi `paid`

Chỉ sau một readback. Indexer thấy payment settle, submitter xác nhận với ledger, và chỉ khi đó
`settle-chain` mới gọi `mark_paid` trên registry. Submit thành công không bao giờ là đủ.

## Đăng nhập

Không có mật khẩu ở đâu cả. Ai đó kết nối ví Stellar và ký một nonce:

1. `POST /auth/wallet/challenge` lưu nonce 32 byte gắn với địa chỉ, network, domain này, và hạn 300
   giây.
2. Ví ký nó. Đó là một **thông điệp**, không phải transaction submit được.
3. `POST /auth/wallet/verify` **lấy** nonce — đọc và xoá trong một câu lệnh — nên không replay được.
   Rồi kiểm domain, network, hạn và chữ ký, theo đúng thứ tự đó.

Mọi thất bại trả cùng một thông điệp sau cùng một độ trễ.

## Những gì chưa nối

- Off-ramp, custody, số dư, job đối soát. Đều ngoài phạm vi; không cái nào có crate.
- Thông báo. Bước S-13 gửi approver một link; trong SOW 1 link đó được copy bằng tay.
- Mọi số liệu tổng hợp cho dashboard. Đó chính là thứ SOW nêu là ngoài phạm vi.

## Những quy tắc code này tuân theo

- **`settle-domain` không phụ thuộc gì**, và không quy tắc nào của nó được viết hai lần.
- **Không crate nào đọc bảng của crate khác.** Các module `settle-db` là ranh giới.
- **Tiền không bao giờ là `number`.** `i128` stroops ở Rust, `numeric(30,7)` ở Postgres, `String`
  trên wire.
- **Kết quả submit không phải câu trả lời.** Mọi kết luận đến từ một readback.
- **`tx_bad_seq` và timeout đi cùng một đường**, và đường đó đọc chain trước.
- **Tính đúng đắn đến từ index**, nên hai bản luôn an toàn.
- **Secret không bao giờ có mặc định.** Một secret mặc định là một secret trong production.
