# 05 · Treasury và multisig

> **Bản dịch tiếng Việt** của [`docs/05-treasury-multisig.md`](../../05-treasury-multisig.md).
> Bản tiếng Anh là bản có hiệu lực.

Module B1–B5. Treasury là một tài khoản Stellar G-account bình thường với 3 signer weight 1 và mọi
threshold đặt 2. Không contract nào giữ tiền; multisig gốc của Stellar làm việc đó, vì nó đã chạy
production nhiều năm và không cần ta audit.

## Vì sao dùng multisig gốc thay vì một contract

Một contract giữ tiền cần security review, một câu chuyện nâng cấp, một cơ chế thoát hiểm, và câu
trả lời cho việc ai chịu trách nhiệm pháp lý với số tiền nó giữ. Không thứ nào vừa 20 ngày, và
không thứ nào là điều SOW yêu cầu. Tài khoản Stellar đã hỗ trợ sẵn weighted signer và threshold
theo từng loại thao tác. Khoảng trống thị trường không phải là nguyên thủy — mà là chưa ai nối
nguyên thủy đó vào một quy trình phê duyệt.

## B2 · Ba signer, threshold 2, master weight 0

| Cấu hình | Giá trị | Vì sao |
|---|---|---|
| Ba signer | weight 1 mỗi người | Tổng weight 3, nên hai bất kỳ trong ba đạt ngưỡng |
| `lowThreshold` | 2 | |
| `medThreshold` | 2 | **Payment là thao tác medium.** Đây là threshold thật sự chặn việc chi tiền |
| `highThreshold` | 2 | Đổi cấu hình signer cũng cần đúng hai người |
| `masterWeight` | **0** | Thiếu bước này thì người giữ secret key của treasury ký một mình vẫn được, và toàn bộ quy trình phê duyệt chỉ là trang trí |

**`masterWeight = 0` là bước hay bị quên nhất, và là thứ đầu tiên reviewer kiểm.** Nó nằm trong
script B2 và được script B5 assert lại. Không bao giờ làm bằng tay.

## B3 · Guard chống khoá tài khoản

Đặt threshold cao hơn weight khả dụng của signer sẽ **khoá tài khoản vĩnh viễn**. `setOptions` và
`accountMerge` là thao tác high-threshold, nên một khi high threshold không đạt được thì không ai
đổi lại cấu hình được nữa. Không có đường phục hồi.

Guard, dưới dạng một vị từ, đánh giá trên **bộ giá trị mới** trước khi submit bất kỳ `setOptions`
nào:

```
Σ weight(signer còn hiệu lực) + masterWeight  ≥  max(lowThreshold, medThreshold, highThreshold)
```

Hai điểm tinh chỉnh quan trọng:

- **Chính xác về cái bị mất.** Payment là thao tác medium. Nếu `medThreshold` vẫn trong tầm weight
  khả dụng mà `highThreshold` thì không, tiền vẫn chuyển được — cái mất là quyền quản trị tài
  khoản. Đó là thảm hoạ chậm hơn, không phải nhỏ hơn.
- **Đừng đặt threshold bằng tổng weight.** Với ba signer, cấu hình 3/3 nghĩa là mất một khoá là mất
  quyền quản trị vĩnh viễn. Ở đây dùng 2/3 chính là để mất một khoá vẫn sống được.

Module B3 có test dựng một cấu hình gây khoá và khẳng định nó bị từ chối (feature F-07). Mọi
`setOptions` trong codebase đều đi qua guard; không chỗ nào tự dựng thao tác đó.

## B4 · Policy và enrollment signer

**Policy được đánh version, không bao giờ sửa tại chỗ.** Đổi signer hay threshold thì ghi một dòng
`governance_policies` mới với `version + 1`; dòng cũ ở lại. Settlement ghim `policy_version` tại
thời điểm authorize, và mọi chữ ký được kiểm với **version đó**, không phải version hiện tại.

Điều này trả lời được câu hỏi mà nhiều tháng sau sẽ không có lời đáp: *lệnh chi này được duyệt theo
luật nào?*

Nó cũng giải quyết chuyện policy đổi giữa lúc đang thu chữ ký. Có hai loại thu hồi và chúng hành xử
khác nhau:

| Thu hồi | Tác động lên chữ ký đã thu |
|---|---|
| Chỉ trong bảng policy của ta | Vẫn hợp lệ trên chain — Stellar không biết chuyện nội bộ của ta |
| Trên chain bằng `setOptions` | Weight tính tại thời điểm apply, nên chữ ký cũ đóng góp 0 và transaction fail `tx_bad_auth` |

Cả hai dẫn tới cùng một kết luận vận hành: **khi policy đổi, mọi settlement đang bay bị huỷ và dựng
lại.** Ghim version là thứ khiến điều đó phát hiện được thay vì thành bất ngờ lúc submit.

## B5 · Xác minh, đồng thời là bằng chứng

Một script đọc lại tài khoản từ Horizon và assert cấu hình. Output của nó nộp làm mốc bằng chứng
**E2**.

| Khẳng định | Kỳ vọng |
|---|---|
| Số signer, không tính master key | 3 |
| Weight mỗi signer | 1 |
| `low_threshold` | 2 |
| `med_threshold` | 2 |
| `high_threshold` | 2 |
| Weight của master key | **0** |
| Có trustline USDC | có |

Nó **assert** chứ không in ra. Một script in cấu hình rồi để người đọc là một script pass trong buổi
demo trong khi master key vẫn còn weight 1.

## Năm cạm bẫy của multisig gốc

**1 · Tranh chấp sequence number.** Chữ ký gắn chặt vào sequence. Hai lệnh chi lấy cùng seq thì lệnh
thứ hai fail `tx_bad_seq` sau khi hai người đã ký. Sequence được khoá ở tầng DB, và chỉ một lệnh
đang bay mỗi treasury.

**2 · Thiếu timebounds.** Một transaction đủ chữ ký nhưng chưa submit là một tấm séc đã ký. Không có
`maxTime`, nó dùng được vô thời hạn bởi bất kỳ ai giữ được bản sao. Luôn đặt cửa sổ hẹp — một giờ —
và thu lại chữ ký nếu hết hạn.

**3 · Master key còn weight.** Đã nói ở trên. Là thứ đầu tiên reviewer kiểm.

**4 · Thu hồi signer giữa chừng.** Đã nói ở B4. Cả hai loại thu hồi đều dẫn tới huỷ và dựng lại.

**5 · Địa chỉ nhận không có trustline.** Gửi USDC tới tài khoản chưa có trustline USDC sẽ fail ở
tầng ledger — nhưng chỉ fail lúc submit, sau khi đã làm phiền cả hai approver. Trustline của địa chỉ
nhận được kiểm lúc tạo lệnh chi và kiểm lại trước khi dựng transaction, không phải sau khi đã thu
xong chữ ký.

## Những quy tắc tài liệu này áp đặt

- **`masterWeight = 0` do script đặt và do script assert.** Không bao giờ bằng tay, không bao giờ
  chỉ một lần.
- **Mọi `setOptions` đi qua guard B3**, đánh giá trên bộ giá trị mới.
- **Threshold không bao giờ bằng tổng weight của signer.** Mất một khoá phải sống được.
- **Policy được đánh version, không sửa.** Settlement ghim version lúc authorize.
- **Policy đổi thì huỷ mọi settlement đang bay.** Cả hai loại thu hồi đều dẫn tới đó.
- **Trustline được xác minh trước khi yêu cầu chữ ký**, không phải sau.
