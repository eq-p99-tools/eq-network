# A carried-stack mass bound without move acknowledgments

A split may increase the occupied stack count although no units leave carried
inventory. The native weight sum counts each stack once. Exact equality at the
projected endpoints therefore cannot establish load across unresolved moves.
The maximum projected prefix alone is also insufficient: an independently refused
merge can leave a stack for later accepted or partial actions.

This bound tracks a finite union of ordinary unreduced root slots touched by
validated projections, and the complete item definitions and quantities originally
inside that union. A later edge grows the union by including its original occupants.
Known whole non-stack relocations may precede a stack split, including neutral
equipment moves. Stack redistribution itself is limited to general slots22-29
and cursor30. Bags, purses, mixed definitions for one ID, unknown definitions,
inter-family projected swaps and external inventory changes invalidate the proof.
An existing unknown/affecting history cannot start it.

For every definition, let Q be its original unit count and S the size of the
touched-slot union. A successful, refused or partially applied ordinary move can
only redistribute those units within its endpoints. It cannot create units or
add a location outside that union. Thus at most min(Q,S) occupied stacks of that
definition can exist under any combination of outcomes. A non-stack definition
keeps its original instance count. Multiply those bounds by the definition's
weight and add the untouched inventory and unchanged coin contribution. Summing
each family's individual maximum may overstate shared slot capacity, which is
safe. This proof needs no acceptance order or positive move acknowledgment.

The slot union can grow only to the existing finite root-slot set; repetition
over the same slots adds nothing. It does not accumulate a per-click penalty.
The result may overstate current load while all placement origins remain pending.
It grants neither equipment/food statistics nor movement permission: the existing
session, stale-state, equipment, food/effect and movement-input guards still apply.

An external slot/quantity/arrival update invalidates the narrow bound, even if
it matches one projection. Existing full Snapshot/Settled resets remain unchanged.
No opcode, sender, packet layout, timeout or dependency is changed here.

Synthetic tests enumerate real-planner outcomes for independently refused and
partially accepted counted actions across repeated cycles, including initially
occupied touched roots. These are offline model regressions, not server receipts
or live compatibility evidence.
