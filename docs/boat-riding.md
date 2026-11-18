# Titanium boat riding and boat zoning

`Capability::BoatRiding` is offered on both Titanium policies, EQEmu and P99.
It speaks the riding wire the Titanium program speaks. No rule tests a race
or a server: every carrier the floor link names counts.

## Naming the boat underfoot

Once a second the session names the boat under the player. This is the
character tick at `eqgame.exe` 0x422198.

- The first tick linked to a carrier sends `OP_BoardBoat` (`0x4298`). Its body
  is the carrier's raw spawn name, its NUL, and three more bytes, which the
  program copies from the spawn record after the name. This client sends
  those three bytes as zero, because their meaning is unread.
- Two unlinked ticks in a row forget the name and send `OP_LeaveBoat`
  (`0x67c9`, empty).
- Whether the last word was a boarding is kept across zones, as the
  program's global is. A rider who zones aboard and finds no carrier
  underfoot therefore leaves once.

No ticks run while an arrival is held (see below) or while a transfer is
pending.

EQEmu reads `OP_BoardBoat` only for a controllable boat, race 141 or 502. For
those it sets the client's controlling boat and says "Board boat: <name>".
`OP_LeaveBoat` clears that controlling boat, and releases the boat's target
if the rider held it without hate. Acquire and release
([boat control](boat-control.md)) stay a separate exchange.

## Zoning with a boat

The server's vehicle transfer (`0x0b71`) is 0x44 bytes: a boat name in 64
bytes, then a u32 zone id. Its case is at 0x45c277. The session matches it
against the saved vehicle name. Both names are cleaned first (digits and `#`
taken out), then compared ignoring ASCII case. When they match, the session
asks for the zone with reason 2, the same request a zone line makes. A
transfer that names another boat, arrives with no saved boat, arrives before
admission, or arrives while a transfer is already pending does nothing.

EQEmu's opcode list does not name `0x0b71`. Its ferry scripts move riders with
an ordinary zone command, which forgets the boat first (0x456110), so the
rider lands at the command's point, as the original client does there.

## The save before leaving

At the approval the program saves the player (`OP_SaveOnZoneReq`, `0x1540`,
0xc0 bytes; 0x4976bf).

- **Boat transfer.** For a boat transfer (reason 2), while the session still
  finds the boat by its name, this client fills the fields the read
  verified: the player's place in the boat's frame at +0x05 and the vehicle
  name at +0x6d. The position goes in the zone request's axis order (y, x,
  z): the program fills both from the same three record fields in the same
  order (0x4976bf, read R36c). The checksum at +0 is 0x4b6c04 over the 0xb8
  bytes from +0x05: the reflected CRC-32 table from a seed of all ones,
  with no final inversion.
- **From the arrival wait.** A transfer that began from the arrival wait
  sends no save at all and keeps the vehicle name. The program skips 0x4976bf
  while it waits.
- **Any other transfer.** The vehicle is forgotten and the save stays the
  empty body ordinary transfers have always sent.

The heading at +0x11 and the other values stay zero. The empty save stays
all zero, checksum included. EQEmu reads none of the body.

## Arriving aboard

The received profile names the vehicle at 0x3378, counted from its checksum.
EQEmu leaves this field empty.

- **Found.** The program looks the name up once, among the spawns already in
  at the profile (0x47981d). Names are cleaned, the spawn's must start with the
  saved one ignoring ASCII case, and the shortest wins. When a spawn is found,
  `WorldEvent::Arrival` carries `Arrival::Aboard`: the name, the found spawn's
  report, and the profile's position in the boat's frame. Movement is held.
  The host stands the player at that point, floor-sweeps from one unit above
  it, retries at (0, 0, 25) in the frame, and answers with
  `GameCommand::ArrivalPlacement` (0x4aba14). The program accepts a floor
  only when it links the named carrier, which only the host can see, so the
  host answers with no position when the floor its sweep found is not that
  carrier's.
- **Placed.** The session takes one placement. It must be within rounding of
  either point across and in heading, no higher than one unit above it, and
  at most 30 units below it. A taken placement corrects the player there and
  publishes `Arrival::Placed`.
- **Not placed.** A placement elsewhere, or none at all (the host found no
  floor), sends the player to the zone's safe point with the vehicle
  forgotten (`Arrival::SafePoint`). This is a local placement, and nothing is
  sent (0x43d7c5). So does a host that gives no answer within 60 s; the
  program places at once, so this bound is this client's.
- **Missing.** When the lookup finds nothing, the session sends `0x3710` (the
  raw saved name and its NUL) and publishes `Arrival::Waiting`. It holds the
  player until the server's `0x0b71` carries them on, or 60 s pass and they go
  to the safe point (0x4a8afa). If the zone refuses that transfer, the
  session forgets the boat and stands the player at the safe point, as the
  program's failed zone request does (0x496d00).

While an arrival is held, moves are refused and no movement is sent. The
network still runs, so camping and quitting work. The player's death forgets
both the boat and any held arrival (0x4564ba).

## Limits

- The 30-unit sweep bound, the rounding bounds and the 60 s bound on a
  host's placement are this client's. The
  program's own sweep reach is unread.
- The `0x0b71` zone request passes the accepted position, where the program
  passes the profile's last saved one.
- `0x0b71`, `0x3710` and a non-empty profile vehicle name have not yet been
  seen in a recording. The path is built from the program read and synthetic
  tests only.
- The program's failed zone request (0x496d00) also uses the safe point for
  ordinary refused requests; this client does so only for a rider carried on
  from the arrival wait. The wider case is an open parity question.
- The program's profile loader (0x4ba0fb) writes the profile's third
  coordinate over the first and leaves the third record field as the
  departure saved it (read R36c, inferred). This client restores the
  position as decoded. Which one a server's boat-frame profile expects is
  open until a recording shows one.
