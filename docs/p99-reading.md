# P99 readable items

P99 offers ordinary Titanium note and book reading. The inventory action derives
the text name and kind from the selected item in the session's current inventory,
with the selection's admission, revision and enqueue time. Missing/stale holdings,
a changed revision, unsupported slot, invalid metadata or expired selection
refuses the read. No item, charge, coin or quest result is predicted.

Titanium's native inventory path (`0x51a543` to `0x4b1db0`) asks for window
`0xff`. The item class is readable (2); the Book byte selects note (0) or book
(1), and Filename supplies the text name. The separate signed BookType field
does not select this Titanium request. EQMac's metadata rule is different and
continues to use its BookType byte. Unknown Titanium flag values are retained
as unsupported metadata rather than converted to book 1.

The request is `OP_ReadBook` (`0x1496`), window, kind and the item's exact
NUL-terminated text name. Length is name length plus three. Names keep the
existing 19-byte server-reference bound, require ASCII and cannot contain an
embedded NUL or denote missing text. There is no free-form filename action.
An archived official P99 note request/reply agrees with the native layout;
book kind 1 is supported by the native program and Titanium source contract.
That historical observation is not live acceptance of this implementation.

The reply supplies window, kind and NUL-terminated server text. The native
Titanium handler (`0x45c617`) routes 0 to the note window and 1 to the book
window, independently of the window byte. Type 2 is item information, and other
types are not silently interpreted as notes. Unsupported types and truncated
text retain a decoder rejection; valid note/book text alone opens a reading
result. Missing text or a successful send does not open one. The wire has no
filename or request identity in its reply, so no per-request acceptance or
causal correlation is fabricated. Existing lossy UTF-8 presentation remains;
complete legacy code-page/typography parity is not established here.

The renderer refuses a read chosen from outstanding inventory projections,
uses the ordered worker revision, and keeps received text separate from local
submission. Death, transfer and disconnect clear reading; text received while
suspended cannot reopen the window. The existing session event ownership still
supplies admission boundaries. Note and book pagination/rendering use synthetic
tests; installed-skin visuals and current P99 live reading remain unverified.

Validation must cover actual P99 policy dispatch, both supported kinds, malformed
names/metadata/replies, stale revision/admission/age, reset and queued projection
refusal. Published tests contain synthetic names and text only; captured content,
private native evidence and installed assets are not repository fixtures.
