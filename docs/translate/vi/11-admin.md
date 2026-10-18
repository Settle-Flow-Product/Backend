# 11 · Bảng điều khiển quản trị

> **Bản dịch tiếng Việt** của [`docs/11-admin.md`](../../11-admin.md).
> Bản tiếng Anh là bản có hiệu lực.

Ứng dụng thứ hai: [`frontend/admin`](../../../frontend/admin/), cổng 5174. Đây là màn hình mà bộ
phận vận hành mở ra khi cần giải thích một việc gì đó — một khoản thanh toán kẹt ở `unresolved`, một
yêu cầu không ai duyệt, một sự kiện on-chain không bao giờ tới.

Nó đọc **PostgreSQL trực tiếp**. Website trong [10-frontend-ux.md](10-frontend-ux.md) đọc qua API
của backend. Khác biệt đó chính là toàn bộ thiết kế.

## Vì sao không đi qua API

Chín endpoint của backend tồn tại để phục vụ luồng thanh toán, và mỗi endpoint là một cam kết mà
website phụ thuộc vào. Một bảng điều khiển vận hành lại đặt những câu hỏi mở — *cho tôi xem mọi
settlement đã giữ chỗ sequence hơn một tiếng trước* — và trả lời chúng qua một hợp đồng HTTP nghĩa
là hoặc thêm một endpoint cho mỗi câu hỏi, hoặc một endpoint đủ tổng quát để trở thành một ngôn ngữ
truy vấn có URL đứng trước. Cách thứ nhất không bao giờ theo kịp; cách thứ hai là một kết nối cơ sở
dữ liệu với nhiều bước thừa và câu chuyện bảo mật tệ hơn.

Vì vậy bảng điều khiển có kết nối cơ sở dữ liệu và không có bề mặt API nào của riêng nó. Không thứ
gì nó làm có thể chạm tới từ ứng dụng phía ví, vì hai bên là hai lần triển khai riêng, trên hai cổng
riêng, với hai tập thông tin đăng nhập riêng.

| | Website | Bảng điều khiển quản trị |
|---|---|---|
| Đối tượng | Người tạo và người duyệt | Vận hành |
| Xác thực bằng | Ví Stellar, theo SEP-10 | Email và mật khẩu |
| Đọc dữ liệu qua | API backend | `pg`, trong cùng tiến trình |
| Nắm giữ | Không thông tin đăng nhập nào | Mật khẩu cơ sở dữ liệu |
| Truy cập được từ | Mạng văn phòng | Chỉ mạng của bộ phận vận hành |
| Phiên nằm ở | Bearer token trong `localStorage` (`settleflow.session.v1`) | Cookie HTTP-only (`settleflow_admin_session`) |

Hai khoá phiên là hai giá trị khác nhau, có chủ đích (`SESSION_KEY`, `ADMIN_SESSION_KEY`). Token
phát cho ứng dụng này không được mở ứng dụng kia, và cách rẻ nhất để bảo đảm điều đó là không bên
nào xác minh được chữ ký của bên còn lại.

Cách lưu trữ khác nhau vì mối đe doạ khác nhau. Token của website do một trang vốn đã trao đổi với
extension ví nắm giữ, và được gửi đi trong header `Authorization` mà mã nguồn chủ động viết. Token
của bảng điều khiển là cookie HTTP-only mà trang hoàn toàn không đọc được — trong bảng điều khiển
không có ví, không có gì cần nhìn thấy token, và một cookie mà script không chạm được là một thứ nữa
mà script bị chèn không lấy đi được.

## Một route cho mọi bảng

Không có một tệp cho mỗi bảng. `app/admin/[table]/page.tsx` là màn hình module duy nhất, và danh
sách bảng lấy từ `pg_catalog` ngay tại thời điểm request.

Viết tay một module cho mỗi bảng thì đúng vào ngày viết ra, rồi lặng lẽ thiếu sót sau lần migration
kế tiếp: bảng mới đơn giản là không xuất hiện, và không có gì báo là nó thiếu. Đọc catalogue nghĩa
là một bảng do migration thêm vào duyệt được ngay khi `make migrate` chạy xong.

Chính sự tổng quát đó cũng là rủi ro. Phần duy nhất của một câu lệnh SQL không thể là tham số ràng
buộc chính là định danh, nên một trình duyệt bảng tổng quát tự thân đã là nguy cơ injection. Quy tắc,
được thực thi trong [`lib/schema.ts`](../../../frontend/admin/lib/schema.ts) và
[`lib/crud.ts`](../../../frontend/admin/lib/crud.ts):

> **Mọi định danh đi vào một câu lệnh đều đã được đọc từ catalogue trước và so sánh bằng đúng
> tuyệt đối. Mọi giá trị đều được ràng buộc.**

Không phần nào khác của bảng điều khiển được phép dựng SQL.

## Những gì bảng điều khiển không hiển thị

| Bảng | Bị giữ lại vì |
|---|---|
| `auth_challenges` | Nonce dùng một lần còn hiệu lực. Liệt kê chúng là phơi bày thông tin xác thực chưa dùng. |
| `admin_users` | Mật khẩu phải được băm trên đường vào, và vai trò quyết định ai được sửa vai trò. Qua một trình ghi tổng quát, một `admin` có thể ghi `password_hash` do chính mình tạo vào hàng của một `superadmin` và trở thành `superadmin`. Được quản lý bằng dòng lệnh thay thế. |
| `_sqlx_migrations` | Sổ sách của trình chạy migration. Sửa nó làm schema và bản ghi của nó lệch nhau. |

Hỏi một trong số này bằng URL sẽ nhận **404**, đúng câu trả lời như khi gõ sai tên bảng — nên không
thể dùng mã trạng thái để suy ra bảng nào có tồn tại.

## Những gì bảng điều khiển không ghi

Quyền đọc thì rộng; quyền ghi thì hẹp, và bị thu hẹp theo *bảng*, không theo người. Không vai trò
nào ghi được vào đường đi của tiền.

| Bảng | Khả năng ghi | Vì |
|---|---|---|
| `settlements`, `settlement_approvals`, `payment_requests` | Chỉ đọc | Đường đi của tiền. Một hàng sửa ở đây làm thay đổi điều mà submitter đang tin về một khoản thanh toán có thể đã đang bay — kể cả số sequence đã giữ chỗ, giá trị duy nhất mà hai khoản thanh toán không bao giờ được dùng chung. |
| `chain_events`, `ingest_cursor` | Chỉ đọc | Là projection, dựng lại từ chain. Sửa một hàng không làm đổi điều đã xảy ra; nó làm đổi điều bảng điều khiển *nói* đã xảy ra, cho tới lần ingest kế tiếp ghi đè lên. |
| `audit_events`, `chain_events` | Chỉ thêm | `REVOKE UPDATE, DELETE` trong migration 0008 mới là chỗ thực thi thật. Bảng điều khiển từ chối chỉ để không mời một nút bấm chắc chắn sẽ thất bại. |
| `governance_policies`, `policy_signers`, `idempotency_keys` | Ghi được | Cấu hình và sổ sách. Không có gì đang bay phụ thuộc vào chúng giữa chừng một khoản thanh toán. |

Một bảng không có khoá chính thì chỉ đọc, bất kể điều gì khác đúng với nó: không có khoá chính thì
không có cách nào gọi tên hàng mà một thao tác sửa nhắm tới.

## Ai được làm gì

Bốn vai trò, quy về ba quyền trong [`types/role.ts`](../../../frontend/admin/types/role.ts) đúng một
lần, thay vì suy lại ở từng chỗ gọi.

| | `viewer` | `operator` | `admin` | `superadmin` |
|---|:--:|:--:|:--:|:--:|
| Đọc mọi bảng hiển thị | ✓ | ✓ | ✓ | ✓ |
| Ghi bảng ghi được | | ✓ | ✓ | ✓ |
| Quản lý tài khoản | | | | ✓ |

Việc kiểm tra diễn ra **bên trong từng server action**, theo từng lời gọi. Không ở layout, và không
ở component đang ẩn một nút bấm. Một server action là một endpoint POST với tên được sinh ra: thứ gì
gọi được thì cũng gọi được bởi người lẽ ra không nên, nên mỗi action đều hỏi ai đang gọi trước khi
làm bất cứ việc gì. Kiểm tra ở layout chỉ ngăn trình duyệt được hiển thị một màn hình nó không thể
lấp đầy; ẩn một nút bấm là phép lịch sự với người dùng, không phải một biện pháp kiểm soát.

`middleware.ts` chỉ kiểm tra rằng cookie phiên *có mặt*. Nó chạy trên Edge runtime, nơi không có cơ
sở dữ liệu — nên nó không thể hỏi tài khoản còn hoạt động hay không, và cũng không giả vờ là hỏi
được. Kiểm tra thật nằm ở `currentCaller()` trong layout và trong mọi action.

## Phiên đăng nhập

Một token HS256 tự viết, trong [`lib/token.ts`](../../../frontend/admin/lib/token.ts) — không thư
viện, và không đọc trường `alg` từ chính token. Thuật toán do mã nguồn quyết định, nên `alg: none`
không phải một câu mà trình xác minh này có thể bị bảo.

Thu hồi là một số nguyên `token_epoch` trên tài khoản. Mỗi token mang theo epoch lúc nó được phát;
`currentCaller()` đối chiếu với hàng trong bảng. Vô hiệu hoá một tài khoản hoặc đổi mật khẩu sẽ tăng
epoch, và mọi token phát trước đó ngừng xác minh được — không cần bảng phiên, và không phải chờ hết
hạn.

Mật khẩu dùng scrypt. Một email không tồn tại vẫn được so với một hash không thể khớp, nên mật khẩu
sai và địa chỉ lạ tốn đúng bằng nhau và trả về đúng cùng một câu chữ. Nếu không, biểu mẫu đăng nhập
trở thành cách để dò xem ai có tài khoản.

## Tài khoản

```
make admin-user ARGS="create ops@settleflow.internal operator"
make admin-user ARGS="list"
make admin-user ARGS="passwd ops@settleflow.internal"
make admin-user ARGS="roles ops@settleflow.internal admin"
make admin-user ARGS="disable ops@settleflow.internal"
```

Vai trò truyền theo vị trí; một tham số không nằm trong bốn tên vai trò sẽ bị bỏ qua, và tài khoản
tạo ra mà không có vai trò nào là `viewer`. `create` và `passwd` hỏi mật khẩu thay vì nhận nó như
một tham số, để mật khẩu không nằm lại trong lịch sử shell.

Không có trang đăng ký và không có kiểu khởi tạo bằng biến môi trường. Cả hai đều là cách để một tài
khoản tồn tại mà không ai quyết định tạo ra nó; tài khoản đầu tiên do người có quyền truy cập cơ sở
dữ liệu tạo, một cách có chủ đích.

`passwd`, `roles` và `disable` đều tăng `token_epoch`, nên mỗi lệnh đều đăng xuất tài khoản đó ở mọi
nơi.

## Tiền trên màn hình

`numeric(30,7)` không vừa trong một number của JavaScript, và `bigint` cũng vậy. Driver `pg` được
yêu cầu để nguyên cả hai dưới dạng chuỗi ([`lib/db.ts`](../../../frontend/admin/lib/db.ts)), còn số
numeric rộng được ép sang `text` ngay trong câu truy vấn thay vì phân tích rồi kết xuất lại.

Quy tắc vẫn là quy tắc mà backend và smart contract giữ: **không có số thực ở bất cứ đâu**. Một bảng
điều khiển làm tròn một stroop khi hiển thị là một bảng điều khiển rồi sẽ bị đem ra trích dẫn trong
một cuộc tranh cãi về việc đã trả bao nhiêu.

## Kiểm thử

`npm test` — 46 trường hợp, không cơ sở dữ liệu và không mạng. Những phần thuần tuý mới là những
phần đáng kiểm thử: mã hoá/xác minh token, thu hồi bằng epoch, so sánh scrypt, quy đổi vai trò, trích
dẫn định danh, xử lý numeric rộng, phân tích tuỳ chọn tìm kiếm, và lời từ chối 404.

`server-only` ném lỗi khi được import ngoài server component, điều này sẽ làm hỏng toàn bộ bộ test,
nên `vitest.config.ts` trỏ nó sang một stub. Đó là lý do bộ mã hoá token nằm trong `token.ts` không
import `server-only`, còn phần I/O nằm trong `session.ts` có import.

Phần còn lại được [12-testing.md](12-testing.md) bao phủ, và được bao phủ bởi chính việc bảng điều
khiển được trỏ vào một cơ sở dữ liệu thật trong buổi demo.
