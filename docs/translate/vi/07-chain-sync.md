# 07 · Đồng bộ chain

> **Bản dịch tiếng Việt** của [`docs/07-chain-sync.md`](../../07-chain-sync.md).
> Bản tiếng Anh là bản có hiệu lực.

Module D1–D4 — bản tối giản. Một thành phần đọc chain; mọi thứ khác đọc thứ mà thành phần đó đã ghi.

## D1 · Ingest theo cursor

Đọc **theo cursor**, không bao giờ theo cửa sổ thời gian. Cửa sổ thời gian âm thầm bỏ sót event khi
đồng hồ trôi hoặc khi một batch chạy lâu hơn cửa sổ.

**Cursor chỉ tiến sau khi commit, không bao giờ trước.** Commit cursor trước khi làm việc nghĩa là
một lần crash mất event vĩnh viễn. Commit sau nghĩa là crash thì đọc lại — và đọc lại an toàn nhờ
D2.

## D2 · Khử trùng tuyệt đối

```sql
CONSTRAINT one_row_per_operation UNIQUE (tx_hash, op_index)
```

Một ràng buộc, và nó mua về ba tính chất:

| Tính chất | Nhờ đâu |
|---|---|
| Xử lý lại một event là no-op | Insert xung đột và vòng lặp đi tiếp |
| Hai bản chạy song song an toàn | Cả hai insert; một thắng; không bản nào làm hỏng dữ liệu |
| Rolling deploy an toàn | Không leader election, không lease, không gì để làm sai khi mạng phân mảnh |

**Không leader election.** Tính đúng đắn đến từ index. Bất cứ thứ gì đòi hỏi đúng một tiến trình
còn sống là một hệ thống hỏng lúc deploy, và deploy xảy ra thường xuyên hơn sự cố.

## D3 · Readback trước khi ghi Paid

Quy tắc mà toàn bộ hệ thống được sắp xếp quanh: **không bao giờ suy ra `paid` từ kết quả submit.**

```
   submit trả Ok            ->  trạng thái: releasing        KHÔNG PHẢI paid
   đọc lại ledger thấy
   transaction đã settle    ->  mark_paid() trên chain
                            ->  trạng thái: paid
```

Submit thành công nghĩa là mạng đã nhận envelope. Nó không nghĩa là transaction được include, và
không nghĩa là nó thành công. Đó là ba sự thật khác nhau, và chỉ cái cuối mới đủ để nói với ai đó
rằng nhà cung cấp của họ đã được trả tiền.

## D4 · Read-model

Giao diện đọc một projection. **Không có lời gọi Horizon đồng bộ nào nằm trong đường request.**

| Đường | Đọc từ | Ngân sách |
|---|---|---|
| `GET /requests` | Read-model | < 45 ms p95 |
| `GET /requests/:id` | Read-model | < 45 ms p95 |
| `GET /requests/:id/evidence` | Read-model | < 45 ms p95 |
| `POST /requests/:id/prepare` | **Horizon, không tránh được** | ~600 ms p95 |

`prepare` là ngoại lệ và không thể khác. Một transaction Stellar mang sequence number phải bằng
sequence hiện tại của tài khoản nguồn cộng một. Con số đó chỉ tồn tại trên mạng; không cache được,
vì bất kỳ transaction nào khác từ tài khoản đó đều làm nó lệch ngay. Một vòng RPC là cái giá trung
thực, và p95 thật của nó được công bố chứ không được che đi.

### Độ trễ được hiển thị, không bị giấu

Nếu read-model tụt quá 60 giây so với ledger mới nhất, giao diện hiện banner "đang đồng bộ" và khoá
nút tạo mới. Số liệu vẫn hiện kèm dấu thời gian mà chúng đúng. Cách khác — hiện số cũ như thể là số
hiện tại — chính là cách ai đó tạo lệnh chi thứ hai cho một hoá đơn đã trả.

## Module này KHÔNG phải là gì

Nó không phải reconciliation indexer. Nó không so số dư, không giữ sổ kép, không phát hiện lệch
giữa chain và database. Những thứ đó thuộc giai đoạn sau và nằm ngoài phạm vi. Việc nó làm là giữ
bốn loại event chảy vào một bảng, không mất và không trùng.

## Những quy tắc tài liệu này áp đặt

- **Đọc theo cursor, không bao giờ theo cửa sổ thời gian.**
- **Cursor tiến sau khi commit, không bao giờ trước.**
- **Khử trùng ở tầng database**, để hai bản luôn an toàn.
- **Không bao giờ bỏ qua một event trong im lặng.** Event không nhận diện được thì cách ly và ghi lại.
- **`paid` đi sau một readback từ ledger**, không phải sau kết quả submit.
- **Không gọi Horizon đồng bộ trong đường đọc.** `prepare` là ngoại lệ duy nhất, và có văn bản.
