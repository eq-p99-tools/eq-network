# eq-network-transport

SOE reliable-UDP framing and sessions used by EverQuest-compatible login,
world, and zone protocols. It owns sequence handling, acknowledgements,
fragmentation, compression, keyed CRCs, retransmission, and session statistics.

Application opcodes and packet bodies belong in the login and game crates.
