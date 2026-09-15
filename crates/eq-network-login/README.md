# eq-network-login

Login application packet codecs for EverQuest-compatible login servers. It
parses and builds authentication, server-list, and world-selection messages and
supports the legacy DES-CBC credential format required on the wire.

Decoded credential owners redact their `Debug` output and zeroize sensitive
buffers on drop. The crate performs no network I/O.
