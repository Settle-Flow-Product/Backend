# 10 · Giao diện

> **Bản dịch tiếng Việt** của [`docs/10-frontend-ux.md`](../../10-frontend-ux.md).
> Bản tiếng Anh là bản có hiệu lực.
> Mockup ASCII của từng màn hình nằm ở bản gốc.

Bốn màn hình, không hơn. SOW ghi rõ dashboard nằm ngoài phạm vi. Mỗi màn hình tồn tại vì có một thứ
trong gói bằng chứng cần nó, không phải vì nó trông hoàn chỉnh.

## Ứng dụng

Hai ứng dụng Next.js App Router:

| Ứng dụng | Cổng | Đối tượng | Cách đọc dữ liệu |
|---|---|---|---|
| [`frontend/website`](../../../frontend/website/) | 5173 | Người tạo và duyệt thanh toán | API backend, qua proxy cùng origin |
| [`frontend/admin`](../../../frontend/admin/) | 5174 | Vận hành — xem hệ thống đã làm gì | **Postgres trực tiếp**, qua server action |

Ranh giới chia theo ranh giới tin cậy chứ không theo sở thích. Website hướng về ví và không giữ
thông tin đăng nhập nào của riêng nó; bảng điều khiển quản trị thuộc về máy chủ, không bao giờ ra
khỏi mạng của bộ phận vận hành, và không có lý do gì để bắt ai đó ký một thứ gì. Gộp chung một
codebase nghĩa là một bundle vừa nói chuyện với ví vừa giữ mật khẩu cơ sở dữ liệu.

Bốn màn hình bên dưới là của **website**. Bảng điều khiển quản trị nằm ở [11-admin.md](11-admin.md).

**Next.js 15 + TypeScript + Tailwind v4 + shadcn/ui.** Các primitive của shadcn nằm ở
`components/ui/` dưới dạng mã nguồn dự án sở hữu, không phải một dependency phải nâng cấp — đó chính
là mục đích của shadcn, và là thứ cho phép `Badge` có thêm biến thể `ok` / `warn` / `err` / `info`
mang ý nghĩa trong nghiệp vụ này. Biến thể dùng `cva`, gộp class qua `cn()`.

Không có `tailwind.config.js`: Tailwind v4 ưu tiên CSS, và bộ token nằm trong `app/globals.css`.

| Route | Màn hình | Module |
|---|---|---|
| `/` | Đăng nhập bằng ví | E1 |
| `/requests/new` | Tạo lệnh chi | E2 |
| `/requests` | Danh sách lệnh chi | E3 |
| `/approve/:id` | Approver ký duyệt | E4 |
| `/evidence/:id` | Bằng chứng on-chain | E5 |

## E2 · Tạo lệnh chi

| Đặc tả | |
|---|---|
| **Validate ngay tại field** | Địa chỉ vendor kiểm regex `^G[A-Z2-7]{55}$`, rồi debounce 500 ms gọi API kiểm account tồn tại và có trustline. Nút disabled cho tới khi có dấu ✓ |
| **Số tiền** | Ô nhập **text**, không bao giờ `<input type="number">`. Tối đa bảy chữ số thập phân. Hiển thị tabular figures |
| **Idempotency** | UUID v4 sinh **khi form mở**, gửi kèm header `Idempotency-Key`. Bấm hai lần tạo một lệnh chi |
| **Sau khi bấm** | Chuyển sang trạng thái "đang ghi lên chain". **Không hiện thành công cho tới khi readback trả về Pending** — bước S-10 |

Kiểm trustline ở đây thay vì lúc submit là thứ ngăn một lệnh chi fail ở ledger sau khi hai người đã
ký. Nó tốn một lời gọi API và cứu một buổi demo.

## E3 · Danh sách lệnh chi

| Đặc tả | |
|---|---|
| **Bốn trạng thái, bốn màu** | Pending vàng, Approved xanh dương, Paid xanh lá, Cancelled đỏ — **luôn kèm chữ**. Màu không bao giờ là kênh duy nhất |
| **Dòng phụ** | Pending hiện tiến độ chữ ký; Paid hiện tx hash rút gọn link ra Stellar Expert; Cancelled hiện ai từ chối |
| **Nguồn dữ liệu** | Read-model. Không gọi Horizon đồng bộ. Nếu độ trễ vượt 60 giây thì hiện banner "đang đồng bộ" và khoá nút tạo mới |
| **Không có gì thêm** | Không biểu đồ, không thống kê, không xuất file. Đó là dashboard, nằm ngoài phạm vi |

## E4 · Approver ký duyệt

| Đặc tả | |
|---|---|
| **Ba thông tin lớn nhất** | Số tiền, địa chỉ vendor, mã tham chiếu. Approver phải xác nhận được cả ba trong 5 giây mà không cuộn |
| **Đếm ngược** | Là timebounds của **transaction** — một giờ — không phải hạn của lệnh chi. Về 0 thì nút ký disabled và màn hình nói cần dựng lại giao dịch |
| **Nút Từ chối** | Cùng cấp thị giác với nút Ký, không giấu đi. Một bước xác nhận rồi mới gọi API |
| **Sau khi ký** | Chưa đủ ngưỡng → màn chờ hiện 1/2. Đủ → "đang gửi giao dịch". **Không bao giờ "đã chi"** |

Phân biệt đếm ngược rất dễ làm sai. Một lệnh chi có thể còn hiệu lực nhiều ngày trong khi transaction
dựng từ nó chỉ còn một giờ. Hiện nhầm cái nào khiến approver tin rằng họ còn thời gian mà thực ra
không.

## E5 · Bằng chứng

**Đây là màn hình quan trọng nhất của SOW 1.** Chapter Lead nghiệm thu bằng chính trang này. Sáu
dòng tương ứng sáu mốc bắt buộc.

| Đặc tả | |
|---|---|
| **Mọi hash đều bấm được** | Link ra Stellar Expert testnet, mở tab mới. Rút gọn 4 đầu – 4 cuối, hover hiện đầy đủ |
| **Nút sao chép** | Xuất văn bản thuần dạng bảng, dán thẳng vào biểu mẫu nộp Instawards |
| **Dòng còn thiếu** | Dấu ○ xám thay vì ✓, để nhìn phát biết còn thiếu bằng chứng nào |

## Design token

Hai họ token, cố ý tách riêng. Bộ token shadcn (`--background`, `--foreground`, `--primary`,
`--border`, `--ring`, …) tạo hình cho giao diện và có cả giá trị sáng lẫn tối. Bốn token dưới đây
mang **ý nghĩa**, và mỗi cái giữ đúng một ý nghĩa ở mọi nơi nó xuất hiện.

| Token | Nghĩa | Trạng thái |
|---|---|---|
| `--ok` | xong | Paid · settled · confirmed |
| `--warn` | đang chờ | Pending · chờ chữ ký · chưa rõ |
| `--err` | dừng lại | Cancelled · rejected · failed |
| `--info` | đang xử lý | Approved · đang gửi |
| `--muted` | không áp dụng | Expired · chưa bắt đầu |

Mỗi token có một cặp `-bg` và `-foreground`, để một badge luôn đọc được ở cả hai theme. Giá trị dùng
`oklch`, đó là thứ giữ cho cặp sáng/tối đều nhau về mặt cảm nhận.

Breakpoint 360 · 768 · 1280.

Số tiền dùng `font-variant-numeric: tabular-nums` để các cột thẳng hàng.

**Màu không bao giờ là kênh duy nhất.** Mọi pill trạng thái đều mang chữ của nó — yêu cầu tiếp cận
cho người mù màu, và nó cũng sống sót khi ảnh chụp màn hình được dán vào biểu mẫu nộp.

## Trạng thái component

| Trạng thái | Khi nào | Hiển thị | Cấm |
|---|---|---|---|
| Loading | Chờ phản hồi đầu tiên | Skeleton đúng hình dạng nội dung sẽ đến | Spinner toàn màn hình |
| Empty | Chưa có dữ liệu | Giải thích và một hành động rõ ràng | Ép tạo ngay sau khi đăng nhập |
| Pending | Đã gửi, chưa có kết quả chung cuộc | `--warn`, đang chờ gì, tx hash nếu có | `--ok`, hoặc chữ "thành công" |
| Unresolved | Timeout hoặc mất phản hồi | Nêu rõ chưa biết kết quả, nút **Kiểm tra lại**, tx hash | Nút **Gửi lại**. Mọi retry tự động |
| Error | Thất bại dứt khoát | Lý do người dùng hiểu được, bước tiếp theo, trace id | Stack trace hoặc mã lỗi trần |
| Terminal | Paid hoặc Cancelled | Trạng thái và giải thích. **Ẩn** mọi hành động | Nút disabled — nó gợi ý rằng có thể mở khoá |
| Degraded | Read-model tụt > 60 giây | Banner "đang đồng bộ"; số liệu kèm dấu thời gian | Cho phép hành động ghi |

Dòng Unresolved là dòng thường bị làm sai. Mời "Gửi lại" sau một timeout chính là cách một nhà cung
cấp được trả tiền hai lần — nên hành động duy nhất là "Kiểm tra lại", và nó đọc lại chứ không gửi
lại.

## Văn bản lỗi — dùng nguyên văn

| Mã | Hiển thị cho người dùng | Hành động |
|---|---|---|
| `WALLET_REJECTED` | Bạn đã từ chối kết nối trong ví. | Thử lại · Đổi ví |
| `NETWORK_MISMATCH` | Ví đang ở mạng khác. Chuyển ví sang Stellar {network} rồi thử lại. | Kiểm tra lại |
| `INVALID_ADDRESS` | Đó không phải một địa chỉ Stellar hợp lệ. | Highlight ô |
| `DESTINATION_NO_TRUSTLINE` | Địa chỉ này chưa thiết lập để nhận {asset}. | Highlight ô |
| `RELEASE_IN_FLIGHT` | Đang có một lệnh chi khác từ treasury này chờ chữ ký. Chờ nó xong đã. | Xem lệnh đang chạy |
| `NOT_A_SIGNER` | Ví này không phải người duyệt của lệnh chi này. | Không có hành động |
| `TX_EXPIRED` | Cửa sổ ký đã đóng. Cần dựng lại giao dịch. | Dựng lại |
| `PAYMENT_UNRESOLVED` | Chúng tôi chưa xác nhận được kết quả. Đừng gửi lại — mở lại trang này sau ít phút. | Kiểm tra lại · Xem giao dịch |
| `SYNCING` | Đang đồng bộ với Stellar. Số liệu tính đến {time}. | Chặn hành động ghi |

Ba quy tắc viết: nói chuyện gì đã xảy ra chứ không nói hệ thống bị gì; luôn kèm một bước tiếp theo
cụ thể, kể cả khi bước đó là "liên hệ người tạo lệnh chi"; và khi một thứ vẫn còn hiệu lực thì nói
ra — đó là câu làm giảm hoảng loạn nhiều nhất.

## Tiếp cận

| | |
|---|---|
| Tương phản | Chữ chính ≥ 7:1, chữ phụ ≥ 4.5:1. Đo bằng công cụ, không bằng mắt |
| Vùng chạm | Tối thiểu 44 × 44 px |
| Bàn phím | Mọi màn hình dùng được bằng bàn phím; focus ring nhìn rõ |

## Quy tắc chung cho cả bốn màn hình

**Không màn hình nào hiển thị thành công trước khi đọc lại từ chain.** Mọi hành động chạm chain trả
`202` và giao diện chuyển sang trạng thái chờ.

## Những quy tắc tài liệu này áp đặt

- **Bốn màn hình.** Màn hình thứ năm cần một dòng trong gói bằng chứng để biện minh.
- **Tiền là chuỗi, render bằng tabular figures.** Không bao giờ ô nhập `number`.
- **Màu luôn có chữ đi kèm.**
- **Đếm ngược của approver là timebounds của transaction**, không phải hạn của lệnh chi.
- **Unresolved mời "Kiểm tra lại", không bao giờ "Gửi lại".**
- **Trang evidence là bề mặt nghiệm thu.** Làm xong nó, đừng để tới Ngày 20.
