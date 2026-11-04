# EQMac notes and books

TAKP and Quarm offer reading through the shared Reading feature and the
generation-owned request/reply codecs. Only a server reply supplies text;
submitting a request neither opens a reading window nor confirms delivery.
This source-derived path still needs integrated and live validation.

The native `SendTextRequestMsg` uses the item's BookType byte at 228 and its
filename at 231. The Book flag at 230 is independent. The request contains
the kind, filename and NUL, with length `filename.len() + 2`. It has no
Titanium window byte. Filename validation keeps the request within the
server's filename array of 20 bytes without truncating a longer name.

Primary server references are
[TAKP e517d0b](https://github.com/EQMacEmu/Server/tree/e517d0bdbca302d94ff6979cd6c6028ddf2b9a4f)
and
[Quarm c2cd265](https://github.com/SecretsOTheP/EQMacEmu/tree/c2cd2651045ab063b236b08f3a9e83cbf10289e9):

- `common/patches/mac_structs.h` and `mac.cpp` keep BookType, Book and
  Filename separate in the 360-byte item record. Only readable-class items
  with a real text name produce a Book descriptor.
- `common/eq_packet_structs.h` defines a one-byte request kind followed by
  a text name of 20 bytes, and a response kind followed immediately by text.
- `Handle_OP_ReadBook` accepts requests up to that structure's size.
  `Client::ReadBook` looks up the name in server data, echoes the kind and
  sends the text when found. An absent text may receive no reply; silence
  does not confirm a read or require retrying.
- `OP_ReadBook` is `ce40` in the existing transport's opcode convention
  (the native/TAKP table's host-order value is `40ce`).

Synthetic regressions cover note/book request bytes and replies, malformed
records, filename bounds, item-field distinctions, both server wires and
local refusals. The client outbox already owns posting-refusal notices;
renderer regressions retain that single visible chat path. Inventory and
its authoritative owner remain unchanged by reading.
