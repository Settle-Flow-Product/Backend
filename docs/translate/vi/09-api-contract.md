# 09 · Hợp đồng API

> **Bản dịch tiếng Việt** của [`docs/09-api-contract.md`](../../09-api-contract.md).
> Bản tiếng Anh là bản có hiệu lực.

Chín endpoint giữa giao diện và backend. Frontend và backend làm song song được từ Ngày 1 nếu chốt
hợp đồng này trước — đó là lý do duy nhất nó được viết trước khi cả hai tồn tại.

## Các endpoint

| Method | Path | Mô tả | Trả về |
|---|---|---|---|
| `POST` | `/auth/wallet/challenge` | Sinh nonce challenge cho một địa chỉ và network | `200` |
| `POST` | `/auth/wallet/verify` | Xác minh chữ ký → phiên làm việc | `200` |
| `POST` | `/requests` | Tạo lệnh chi. **Bắt buộc header `Idempotency-Key`** | `202` |
| `GET` | `/requests` | Danh sách, kèm trạng thái và tiến độ chữ ký | `200` |
| `GET` | `/requests/:id` | Chi tiết một lệnh chi | `200` |
| `POST` | `/requests/:id/prepare` | Dựng tx chưa ký, khoá sequence, trả XDR + tx hash + `valid_until` | `200` |
| `POST` | `/requests/:id/signatures` | Approver nộp chữ ký trên tx hash | `200` |
| `POST` | `/requests/:id/reject` | Từ chối. Giải phóng sequence, gọi `cancel_request` | `202` |
| `GET` | `/requests/:id/evidence` | Bảng tx hash từng mốc cho trang bằng chứng | `200` |

Mọi thứ treo dưới `/requests/:id`. Không có resource settlement riêng trong API, vì trong SOW 1 một
lệnh chi có nhiều nhất một settlement, và phơi cả hai sẽ mời giao diện theo dõi hai định danh cho
cùng một thứ.

## Vì sao ghi lên chain trả 202

**Mọi endpoint chạm chain trả `202 Accepted`, không phải `200 OK`.**

Đây là cách hợp đồng API đẩy nguyên tắc "chain là nguồn sự thật" xuống tận cái nút bấm. Không có
đường nào để client hiểu "API trả OK" thành "tiền đã chuyển". Nếu một endpoint chạm chain trả `200`,
đó là lỗi thiết kế, không phải sở thích.

`POST /requests` và `POST /requests/:id/reject` trả `202`. `prepare` và `signatures` trả `200`, vì
cả hai không submit gì — `prepare` đọc mạng và `signatures` ghi một dòng.

## Xác thực

| Tiền tố | Tư thế |
|---|---|
| `/auth/*` | Công khai. Đây là nơi phiên làm việc sinh ra |
| mọi thứ còn lại | Một phiên từ `/auth/wallet/verify`, gửi kèm `Authorization: Bearer …` |

Challenge là một **thông điệp**, không phải một transaction submit được. Thiết kế nào bắt người ta
ký một transaction thật để đăng nhập là thiết kế mà ví có thể bị lừa ký một lệnh chi.

### Challenge là một transaction SEP-10 với sequence number 0

`POST /auth/wallet/challenge` trả về một transaction, và ví ký nó bằng `signTransaction`.

**Ký nó an toàn vì nó không bao giờ submit được.** Sequence number của một transaction phải bằng
sequence hiện tại của tài khoản nguồn cộng một; số 0 không bao giờ là con số đó. Ví hiển thị một
transaction, người dùng phê duyệt, và kết quả là một thứ về mặt cấu trúc không tiêu được.

"Đừng bao giờ bắt ai ký một transaction để đăng nhập" là một quy tắc quá thô. Thứ không bao giờ được
ký là một transaction **submit được**.

> **Cách này thay cho thiết kế `signMessage` vốn không chạy được.**
>
> `signMessage` không tương thích giữa các ví. Các ví bất đồng về việc chúng thực sự ký cái gì — byte
> thô, `sha256` của chúng, hay byte kèm một tiền tố phân tách miền — và Freighter đưa nó qua đường
> "sign blob" cũ vốn coi payload là một khối mờ. Backend không thể verify một chữ ký khi nó không
> biết cái gì đã được ký, và thất bại đó không phân biệt được với khoá sai. `signTransaction` là
> thao tác duy nhất mọi ví cài đặt giống hệt nhau, vì làm sai nó sẽ hỏng việc chi tiền.

**Việc verify không bao giờ tin transaction được trả về.** Chỉ chữ ký được đọc ra; challenge kỳ vọng
được dựng lại từ nonce, địa chỉ và thời điểm phát hành **đã lưu**, rồi chữ ký được kiểm trên hash
đó. Nếu không, người gọi có thể ký một transaction do chính họ chọn và được chấp nhận.

### Mọi lỗi trả cùng một phản hồi, và ghi log một lý do khác nhau

Không có gì client nhìn thấy phân biệt được "không có challenge" với "chữ ký sai"; khác biệt đó sẽ
nói cho kẻ tấn công biết một địa chỉ có được ghi danh hay không.

Nhưng **lý do luôn được ghi log**: `nonce_not_32_hex_bytes`, `challenge_not_found_or_replayed`,
`domain_mismatch`, `network_mismatch`, `signature_not_hex_or_base64`, `bad_signature`,
`challenge_expired`. Một bản trước đó thoát khỏi hai trong số các nhánh này mà không ghi log, và kết
quả là một lần đăng nhập thất bại mà không bên nào tìm ra được lý do.

## Cấu trúc lỗi

Mọi lỗi, từ mọi endpoint, có cùng một hình dạng:

```jsonc
{
  "error": {
    "code":      "RELEASE_IN_FLIGHT",   // máy đọc, ổn định, không bao giờ đổi tên
    "message":   "Đang có một lệnh chi khác từ treasury này chờ chữ ký.",
    "field":     "amount",              // để giao diện highlight đúng ô
    "details":   { "settlement_id": "0193f2b8-..." },
    "retriable": false,
    "trace_id":  "01J8XKQ2..."          // đưa cho support
  }
}
```

`code` ổn định và máy đọc được; `message` dành cho người và có thể dịch. `field` là thứ khiến S-08
thực hiện được — lỗi hiện ngay dưới ô gây ra nó thay vì trong một toast chung chung.

## Mã trạng thái HTTP

| Mã | Khi nào | Client làm gì |
|---|---|---|
| `200` | Một lần đọc, hoặc một lần ghi không chạm chain | Render |
| `202` | Đã nhận, chưa biết kết quả — tạo lệnh chi, từ chối | Chuyển sang trạng thái chờ, poll. **Không bao giờ hiện thành công** |
| `400` | Validate thất bại | Highlight field. Không retry |
| `401` | Không có phiên, hoặc phiên hết hạn | Đăng nhập lại |
| `403` | Không phải signer của settlement này | Giải thích; không mời retry |
| `404` | Không tìm thấy — kể cả khi có nhưng không thuộc về bạn | Kết thúc. Không dò tiếp |
| `409` | Idempotency đang bay, hoặc `RELEASE_IN_FLIGHT` | Chờ rồi poll. **Không đổi idempotency key** |
| `422` | Trạng thái hiện tại không cho phép hành động này | Đọc lại entity và cập nhật giao diện |
| `429` | Rate limit | Backoff theo `Retry-After` |
| `503` | Read-model tụt quá ngưỡng | Hiện "đang đồng bộ", chặn hành động ghi |

`404` bao gồm cả "có nhưng không phải của bạn" một cách có chủ đích. Phân biệt hai ca nói với một
người gọi không có quyền rằng một request id là có thật.

`409` trên idempotency nghĩa là cùng key đó đang được xử lý. Hành vi đúng của client là chờ rồi poll
với **cùng key**; sinh key mới chính là cách tạo ra một bản trùng.

## Mã lỗi

| Mã | Ý nghĩa |
|---|---|
| `INVALID_ADDRESS` | Địa chỉ vendor không phải public key Stellar hợp lệ |
| `INVALID_AMOUNT` | Bằng 0, âm, hoặc quá bảy chữ số thập phân |
| `ASSET_NOT_SUPPORTED` | Không phải USDC hay XLM |
| `INVALID_REFERENCE` | Rỗng, hoặc dài quá 64 ký tự |
| `IDEMPOTENCY_CONFLICT` | Cùng key, khác payload |
| `DESTINATION_NO_TRUSTLINE` | Vendor chưa thiết lập để nhận tài sản này |
| `RELEASE_IN_FLIGHT` | Đang có một lệnh chi từ treasury này chờ chữ ký |
| `NOT_A_SIGNER` | Địa chỉ không thuộc policy đã ghim vào settlement |
| `BAD_SIGNATURE` | Chữ ký không verify được trên tx hash này |
| `ALREADY_SIGNED` | Signer này đã ký settlement này rồi |
| `TX_EXPIRED` | Hết timebounds; phải dựng lại và thu lại chữ ký |
| `NOT_PAYABLE` | Lệnh chi đã paid, cancelled hoặc expired |
| `SYNCING` | Read-model đang tụt; hành động ghi bị chặn |

Mã là định danh ổn định. Đổi tên một mã sẽ phá bảng văn bản của giao diện và mọi runbook vận hành
ánh xạ mã sang hành động.

## Những quy tắc tài liệu này áp đặt

- **Ghi lên chain trả 202.** Một endpoint chạm chain mà trả 200 là một lỗi thiết kế.
- **Số tiền qua wire dưới dạng chuỗi.** Số trong JSON là double.
- **`Idempotency-Key` bắt buộc với `POST /requests`**, và `409` không bao giờ có nghĩa "dùng key mới".
- **Hình dạng lỗi giống hệt nhau ở mọi nơi**, kể cả `trace_id`.
- **`404` che giấu sự tồn tại.** Không-phải-của-bạn và không-tìm-thấy là cùng một phản hồi.
- **Mã lỗi là định danh ổn định**, không phải văn xuôi.
