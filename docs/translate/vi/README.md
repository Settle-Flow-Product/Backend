# SettleFlow — tài liệu xây dựng SOW 1

> **Bản dịch tiếng Việt.** Bản gốc tiếng Anh nằm ở [`docs/`](../../).
> Khi hai bản mâu thuẫn, **bản tiếng Anh là bản có hiệu lực** — đó là bản mà mã nguồn và bộ test được đối chiếu.

SettleFlow SOW 1 là quy trình chi tiền cho nhà cung cấp có bước phê duyệt nội bộ, trên Stellar.
Doanh nghiệp ghi một lệnh chi lên contract sổ đăng ký Soroban — địa chỉ vendor, số tiền, loại tài
sản, mã tham chiếu — và theo dõi nó qua `pending → approved → paid`. Backend dựng một transaction
Stellar chưa ký từ lệnh chi đã approved, thu chữ ký của từng approver, rồi submit khi tổng weight
đạt ngưỡng của tài khoản treasury.

**Contract không bao giờ giữ một đồng nào.** Nó ghi nhận và xác minh. Đó là lý do nó lên được
testnet trong 20 ngày làm việc mà không cần security audit, và điều đó được kiểm bằng máy ở mỗi lần
push chứ không phải bằng trí nhớ.

## Phạm vi của tài liệu này

Chỉ **SOW 1** — 20 ngày làm việc theo hợp đồng, 3 deliverable, $5.000. Nó không mô tả toàn bộ sản
phẩm SettleFlow; contract giữ tiền, off-ramp, dashboard và cổng nhà cung cấp đều nằm ngoài phạm vi
và chỉ xuất hiện ở cột "không làm".

Mọi thứ ở đây bắt nguồn từ một file:
[`../../../resources/specs/settleflow-sow1-deck.html`](../../../resources/specs/settleflow-sow1-deck.html).

## Nguồn sự thật

| Câu hỏi | Được trả lời bởi | Không phải bởi |
|---|---|---|
| Lệnh chi này đang ở trạng thái nào? | `get_request` trên registry contract | Database — nó chỉ soi chiếu |
| Phê duyệt này đã đủ chưa? | Threshold của treasury trên chain | Bảng `policy_signers` — nó ghi ý định |
| Tiền đã chuyển chưa? | Đọc lại từ ledger | Kết quả `submit` — hai chuyện khác nhau |
| Mọi thứ còn lại | PostgreSQL | — |

## Thứ tự đọc

| # | Tài liệu | Nội dung |
|---|---|---|
| 00 | [00-overview.md](00-overview.md) | Vấn đề, ba deliverable, và ranh giới phạm vi cứng |
| 01 | [01-features.md](01-features.md) | F-01…F-13, mỗi feature có cách nghiệm thu quan sát được |
| 02 | [02-modules.md](02-modules.md) | 42 module trong 7 nhóm — chính là issue tracker |
| 03 | [03-flow.md](03-flow.md) | S-01…S-30, và máy trạng thái của một lệnh chi |
| 04 | [04-contract-interface.md](04-contract-interface.md) | `payment_request_registry`: kiểu, thứ tự gate, mã lỗi, event, TTL |
| 05 | [05-treasury-multisig.md](05-treasury-multisig.md) | Cấu hình treasury, `masterWeight = 0`, guard chống khoá |
| 06 | [06-signature-service.md](06-signature-service.md) | Dựng tx, khoá sequence, thu chữ ký, submit, reconcile |
| 07 | [07-chain-sync.md](07-chain-sync.md) | Ingest theo cursor, khử trùng, readback trước khi ghi `paid` |
| 08 | [08-data-model.md](08-data-model.md) | Những bảng thật sự cần, và các index mang tính đúng đắn |
| 09 | [09-api-contract.md](09-api-contract.md) | Chín endpoint, cấu trúc lỗi, và vì sao ghi lên chain trả 202 |
| 10 | [10-frontend-ux.md](10-frontend-ux.md) | Bốn màn hình, design token, trạng thái component |
| 11 | [11-admin.md](11-admin.md) | Bảng điều khiển quản trị: một route cho cả catalogue, và những gì nó từ chối ghi |
| 12 | [12-testing.md](12-testing.md) | 16 ca contract, integration trên testnet thật, chaos test |
| 13 | [13-deployment.md](13-deployment.md) | Deploy, cấu hình, xác minh, biến môi trường |
| 14 | [14-roadmap.md](14-roadmap.md) | 20 ngày trong 5 chặng, mỗi chặng có đầu ra đo được |
| 15 | [15-evidence.md](15-evidence.md) | Bảy mốc bằng chứng, và Definition of Done |
| 16 | [16-risks.md](16-risks.md) | Sáu rủi ro kèm phương án, và ba câu cần chốt trước Ngày 1 |
| — | [backend/README.md](backend/README.md) | Backend chạy thế nào, theo đúng thứ tự code thực thi |

## Hình dạng SOW 1 trên một trang

```
  Deliverable 1   35% · $1.750    payment_request_registry            Soroban · không custody
                                  create_request     creator ký
                                  approve            settler ký · pending -> approved
                                  mark_paid          settler ký · approved -> paid
                                  cancel_request     creator ký · nhánh từ chối
                                  get_request        chỉ đọc, qua RPC simulation
                                  is_payable

  Deliverable 2   35% · $1.750    treasury            G-account · 3 signer weight 1
                                                      threshold low/med/high 2 · masterWeight 0
                                  dịch vụ chữ ký      build_unsigned  giữ MỘT sequence của treasury
                                                      submit_signature  ed25519 trên đúng hash
                                                      đủ threshold -> ghép -> submit
                                                      reconcile_then_decide  đường duy nhất dựng lại

  Deliverable 3   30% · $1.500    demo round trip     lệnh chi -> 2/3 chữ ký -> chi cho vendor
                                                      -> registry đánh dấu paid · ghi lại mọi hash

  Tài sản         USDC (Stellar Asset Contract) và XLM native · 7 thập phân · i128 stroops
  Tiền            numeric(30,7) ở Postgres · i128 ở Rust · String trên wire · không float ở đâu cả
  Thời gian       env.ledger().timestamp() quyết định hết hạn. Đồng hồ server không quyết định gì.
  Thành công      chỉ sau readback. Transaction đã submit không phải là lệnh chi đã trả.
  Quy mô          13 feature · 42 module · 30 bước · 4 màn hình · 7 mốc bằng chứng
```

## Những quyết định khác với deck

| Deck nói | Chúng ta làm | Vì sao |
|---|---|---|
| Sản phẩm thương mại sẽ thu tiền từ khách; SOW mô tả chi tiền cho vendor | Chỉ chi tiền cho vendor | Chính deck đã chốt: sổ đăng ký và cơ chế phê duyệt là cùng một thứ cho cả hai chiều, nên code giao vẫn đúng nguyên vẹn. Evidence được đối chiếu với văn bản đã ký. |
| `app.settleflow.internal` với bốn màn hình | Hai ứng dụng Next.js: `frontend/website` và `frontend/admin` | Ranh giới chia theo ranh giới tin cậy, không theo sở thích. Website hướng về ví và không giữ thông tin đăng nhập nào; bảng điều khiển giữ mật khẩu cơ sở dữ liệu và không bao giờ ra khỏi mạng vận hành. Một codebase chung nghĩa là một bundle vừa nói chuyện với ví vừa giữ mật khẩu. |
| Backend, không nêu ngôn ngữ | Cargo workspace bằng Rust ở `backend/crates/` | Cùng toolchain với contract, một lint gate, một lệnh test. |
| `PaymentRequest` có chín trường | Đúng chín trường, không thêm | Mỗi trường thừa trên chain là storage rent không ai dự trù và một migration không ai lên kế hoạch. |

## Những chỗ deck bỏ trống mà đặc tả này lấp

| Chỗ trống | Được lấp ở |
|---|---|
| 12 mã lỗi được nêu tên nhưng không đánh số, không có thứ tự | [04-contract-interface.md](04-contract-interface.md) |
| Thứ tự đánh giá gate không được nêu, nhưng test khẳng định mã lỗi chính xác | [04-contract-interface.md](04-contract-interface.md), [12-testing.md](12-testing.md) |
| Guard chống khoá tài khoản chỉ là văn xuôi, chưa thành một vị từ kiểm được | [05-treasury-multisig.md](05-treasury-multisig.md) |
| Sáu bảng được nêu tên nhưng chưa định nghĩa, chưa index, chưa xếp thành migration | [08-data-model.md](08-data-model.md) |
| Hình dạng response lỗi không được đặc tả | [09-api-contract.md](09-api-contract.md) |
| "16 unit test" là một con số, không phải một danh sách | [12-testing.md](12-testing.md) |
| Tổng số module deck ghi (37) lệch với chính các con số theo nhóm của nó (42) | [02-modules.md](02-modules.md) |

## Trạng thái repository hiện tại

- Đặc tả đã viết; code đã dựng khung.
- Chưa deploy gì lên testnet. [15-evidence.md](15-evidence.md) giữ bảng tx hash rỗng để buổi demo
  điền vào.
