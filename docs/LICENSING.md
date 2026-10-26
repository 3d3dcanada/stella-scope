# Stella Scope — licensing design note

Stella Scope's source is public under AGPL-3.0 (published as 0.4.2 on `main`, `3d3dcanada/stella-scope`).
This note explains how 3D3D, as copyright holder, can also sell a commercial farm licence for Stella Scope,
and what the AGPL lets anyone do. It is plain language, not legal advice; every point marked "lawyer" is a
question a Canadian lawyer should confirm before 3D3D relies on it.

## Who owns the code

Essentially all of Stella Scope's code is written by 3D3D Canada (`3D3D Canada` and `3d3dcanada` in git;
`3d3dcanada/stella-scope`). A handful of commits are from `Stella Scope build` and `Claude Opus 5` (build
and tooling commits made on 3D3D's behalf). The dependencies are all permissively licensed: serde,
serde_json, rusqlite, ureq, getrandom, qrcode, ed25519-dalek and image (MIT / Apache-2.0 / ISC). None is
AGPL or GPL, so none restricts 3D3D from selling a commercial licence on top of its own code.

## What the AGPL-3.0 source is

The published source (AGPL-3.0) is Stella Scope complete: the service, the web UI, the collectors, the
drivers and the farm queue. Anyone may:

- read it, build it and run it themselves, for any number of printers;
- modify it and run their modified build;
- share it, modified or not, under AGPL-3.0 (which passes the source obligation onward).

The AGPL network clause means someone who runs a modified Stella Scope and lets others use it over a
network must offer them the corresponding source. That is the AGPL working as intended.

## Why selling a farm licence is acceptable

The AGPL is a copyright licence. It grants recipients rights; it does not take rights away from the
copyright holder. 3D3D, as copyright holder, is not bound by its own AGPL grant to others and may sell a
separate commercial licence for the official builds. A farm licence is:

- a commercial right to run more than five printers with the official Stella Scope builds, plus support;
- sold by 3D3D for CAD, bought by writing to info@3d3d.ca and paid by Interac e-Transfer from an invoice (the payment details come with the invoice);
- issued offline as a signed text (`SFL1....`), checked by the program against a public key compiled into
  the build. Nothing phones home, no account, no tracking.

The first five printers remain free under the AGPL build; the licence is what unlocks the sixth printer and
beyond in the official, supported build.

## What a person can do without a licence

Build Stella Scope from the AGPL source and run it with any number of printers. The five-printer free tier
and the licence check exist in the official builds; someone who builds from source can remove or bypass the
check entirely. That is acceptable: it is what AGPL-3.0 permits, and the farm licence is sold as support and
convenience (official builds, signed licences, one business day issuing), not as access to code that is not
already public.

## What 3D3D does not own (third-party code and models)

- `docs/THIRD_PARTY.md`: print-farm-manager (MIT, idea only, no code copied) and Bambuddy (AGPL, feature
  ideas only, no code or assets taken).
- `docs/PRINTER-MODELS-NOTICE.md`: the 67-machine catalogue from 3D Print Rig (MIT) is carried as facts
  (names, sizes, SHA-256 pins, each machine's own licence) — no CAD. Carried machine pictures are open
  licence only. GPL-3.0 machine models are never carried with the build; they are fetched by the person on
  request under their author's terms.

None of this is 3D3D's code, and none of it prevents 3D3D from licensing its own Stella Scope code.

## Points a Canadian lawyer should confirm (not legal advice)

- Whether the commercial farm licence should be offered under New Brunswick law and whether the AGPL-3.0
  public source and the commercial licence can coexist cleanly for Stella Scope's own code.
- Whether the Interac e-Transfer-only payment method and the "licence by email within one business day"
  commitment create any consumer-protection obligations.
- The exact boundary between "five printers free" and "sixth printer requires a licence" for a farm that
  adds and removes printers over time (trial, expiry, re-licensing).
- Whether demo printers (simulations on the person's own machine) correctly never count toward the five.

This note does not claim legal advice. 3D3D should have a Canadian lawyer review `docs/LICENSING.md` and
`docs/FARM-LICENCE-TERMS.md` before selling a licence.
