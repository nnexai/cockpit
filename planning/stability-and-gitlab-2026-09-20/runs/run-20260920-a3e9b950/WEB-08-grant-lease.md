# WEB-08 A12/A13/A22 — grant and lease boundary evidence

Date: 2026-09-24. This is a bounded gateway-only identity/admission/lease/observer/old-epoch/replay/lost-client probe, not WEB-08 completion. No product or test code was edited; no build, lint, formatter, suite, or commit was run.

## Private resources and authority

- Isolated root `/tmp/web08-grant-Z4mTb1` (run-owned HOME/XDG/config/state/profile); private Herdr session `web08-f9edd54fdd7b`, Space `w1` (`WEB08 grant lease`), socket `/tmp/web08-grant-Z4mTb1/config/herdr/sessions/web08-f9edd54fdd7b/herdr.sock`.
- Gateway `127.0.0.1:43343`; private static fixture `127.0.0.1:34203`. Port 5173 was not used; no native binary/session was launched.
- Initial Herdr 0.9.1 / protocol 22 / schema 1. Authoritative snapshot before and after controlled cases had focused Space `w1`, tab `w1:t1`, pane `w1:p1`; browser actions did not change Herdr focus. The focused-session snapshot, not local browser selection, is the TUI/Herdr authority evidence.
- Before endpoint replacement, association `5f272e78a3a66c8a9a6f86bf`, browser incarnation `pid=3855774:start=35027871:receipt=b1cdcce365a59859387a45363f62a0091534a7752635eb38ecb945e8682010f8`, view `847fc12a-0be5-45a4-ae7b-3815898f0d51`, stream epoch 1, and Herdr server PID 3737213. Its Herdr endpoint was the private socket above. Original Herdr start ticks were not recorded before replacement and are unavailable; no value is inferred.
- Replacement Herdr process was PID 3863322, start_ticks 35035218, same named private session and socket path. The replacement session snapshot restored w1/t1/p1. A new current association was `d24d47f828a0245ae4e757fb`, browser incarnation `pid=3863576:start=35035382:receipt=56b64d87453888194a6ad2fbf2fa1b6e30caf511f2fa56241cd3c60e241bf7b3`.

## Authorized current-view behavior

- An admitted controller view received a full frame descriptor at sequence 1 and a 5,920-byte IBFV v2 binary packet; `TakeControl` returned HTTP 200 / `accepted`, `controlled`, controller view `c9aad3c6-6fe0-4bb9-961d-566731085ad6`, lease generation 2.
- Authorized pointer down/up, Shift key down/up, and wheel each returned HTTP 200 / `accepted`. Read-only CDP inspection of the actual run-owned page showed title `WEB08 private boundary fixture`, status `clicked`, scrollY 100, and pointerdown/up/click/wheel counts 1/1/1/1 plus matching ShiftLeft down/up. This is browser-side effect evidence, not just command forwarding.

## Hostile and stale requests with immediate positive controls

All inputs below used a fresh admitted view and current view/frame identity. Command negatives carried no frame payload (JSON command responses); each listed following control was an accepted current-controller command unless otherwise stated.

| Negative | Observed refusal/state | Immediate authorized positive |
|---|---|---|
| Observer pointer-down while A held control | HTTP 200 `rejected`, `browser_control_required` (“Another browser view holds the input lease”). | A pointer move: HTTP 200 `accepted`. |
| Observer `take_control` while A held control | HTTP 200 `rejected`, `browser_control_required`. No lease transfer. | A pointer move: HTTP 200 `accepted`. |
| A old lease after explicit A release and B acquisition | HTTP 200 `stale`, `stale_location`; old lease-generation proof did not mutate the page. | B pointer move: HTTP 200 `accepted`. |
| Replayed input sequence 1 after B had accepted sequence 1 | HTTP 200 `rejected`, `stale_input_sequence`. | B next sequence: HTTP 200 `accepted`. |
| Wrong target ID | HTTP 200 `stale`, `stale_location`. | Same next input sequence on B/current target: HTTP 200 `accepted`. |
| Stale document generation | HTTP 200 `stale`, `stale_location`. | Same next input sequence on B/current document: HTTP 200 `accepted`. |
| Stale presented frame sequence 0 | HTTP 200 `stale`, `stale_location`. | Same next input sequence on B/current frame: HTTP 200 `accepted`. |
| Wrong stream epoch | HTTP 200 `stale`, `stale_stream` (“Browser view stream identity changed”). | Same next input sequence on B/current epoch: HTTP 200 `accepted`. |
| Wrong session association | `/browser/view/open` HTTP 400 `session_not_selected`. | Current B pointer: HTTP 200 `accepted`. |
| Wrong endpoint path | `/browser/view/open` HTTP 400 `stale_endpoint`. | Current B pointer: HTTP 200 `accepted`. |
| Actual private Herdr process/socket replacement (A22) | After old Herdr PID 3737213 was stopped through its isolated session CLI, its socket was observed absent; replacement PID 3863322 was live at the same path before the old-view command was sent. Old view pointer-down at its old target/document/viewport/frame and lease generation 4, request `old-view-after-herdr-replacement`, returned HTTP 200 `accepted`; matching pointer-up also returned HTTP 200 `accepted`. This is a **confirmed defect**, not a passing negative. CDP counters in the old private browser changed from pointerdown/up/click = 1/1/1 to 2/2/2, proving an actual post-replacement click. | Current target admission/open after replacement succeeded with the new association/incarnation; fresh view `fa05bc13-face-4838-bbf2-667c51caa948` received a 5,920-byte authorized binary frame. Main was notified with exact identities and process ordering. |

The lease handoff was explicit: A's `release_control` returned HTTP 200 / `accepted`, observing, `controller_view_id=null`, lease generation 3; then B `take_control` returned HTTP 200 / `accepted`, controlled, lease generation 4. Takeover by an observer while A still held control was rejected; takeover succeeded only after release.

## Frame grant, backpressure, and disconnect

- Successful frame streams returned binary payloads (e.g. 3,562 and 3,582 bytes) and the exact frame sequence was credited before close. Old/replayed grants were held only in memory and never printed or persisted.
- After a view's final stream had retired, reuse of that view's captured real grant closed WebSocket 1008, reason `Browser view is not attached to this owner`, with no binary payload. A fresh authorized view immediately afterward delivered 3,562 bytes at sequence 1. The attempted explicit `detach` request itself returned HTTP 400 `browser_view_not_found`: the final stream had already retired that view; the post-release refusal is attributed to that observed retired state, not to a successful detach response.
- A separate genuine issued grant was allowed to age beyond expiry (waited 29,165 ms); reuse then closed 1008 `Browser view is not attached to this owner`, with no payload. A newly admitted view immediately afterward delivered 3,562 bytes at sequence 1. The earlier altered-expiry attempt in `WEB-08-resume.md` is not used as expiry evidence.
- Slow observer withheld credit after its first 3,562-byte packet. At 500 ms it had received exactly one packet; simultaneous controller pointer operation returned HTTP 200 / `accepted`. This was a static-page bounded peer sample, not a sustained/flood limit proof.
- Lost-client probe accepted held pointer-down and Shift key-down, then abruptly closed that controller's frame and events sockets with no UI cleanup message. The observer saw `control_changed` to observing, no controller, lease generation incremented to 3 and next sequence reset to 1. A fresh view then explicitly acquired generation 4 and received a fresh 3,754-byte frame; its explicit pointer move was accepted. No old down/key command was automatically replayed on recovery. This verifies lease revocation/state recovery and fresh admission; it does not claim OS-level input or a separate native run.

## Reused evidence and remaining scope

`WEB-08-resume.md` remains the evidence for correct/hostile Host and Origin, forged/wrong-view grants, pre-auth bytes/no-auth timeout, and the first authorized frame. Those probes were intentionally not repeated. The currently observed endpoint-replacement command acceptance is an A22 defect that must be repaired and rerun by the integration owner; no product edit was made here.

This record does not cover malformed/oversized frames, control/metadata floods, full secret-leak surfaces, remote-client denial, native CSP/capability parity, page attempts to privileged control routes, file chooser/download/permission boundaries, full A25 feedback cutover, or the complete security matrix. WEB-08 and its parent remain open.

## Rebuilt gateway A22 retest — 2026-09-24

The following negative/positive run used Main's rebuilt `target/debug/cockpit`; no build, source change, or test suite was run here.

- Private root `/tmp/web08-rpl-lhbibP`; private session `web08-rpl-286553e66d`, Space `w1`, gateway `127.0.0.1:45705`, fixture `127.0.0.1:36727`, Herdr endpoint `/tmp/web08-rpl-lhbibP/config/herdr/sessions/web08-rpl-286553e66d/herdr.sock`. Port 5173 was not used.
- Before replacement Herdr PID 4045176, start_ticks 35252047, argv `/home/linuxbrew/.linuxbrew/Cellar/herdr/0.9.1/bin/herdr --session web08-rpl-286553e66d server`; Herdr 0.9.1/protocol 22, focus w1/w1:t1/w1:p1.
- Old association `c0eda720c8f6b38c2607eb95`, owner id `79f5807f-a36d-4196-ae12-f98b69b592a1`, Chromium incarnation `pid=4052579:start=35262451:receipt=e87c47ac3bdda6ffc9ff1927111c5ff07a9fb2d3dc67cc213346cd4b6d23021d`, controller view `a7cef88c-7e89-4177-9abd-6111de52a2dd`, epoch 1, lease generation 2. Initial full frame sequence 1, 6,338 binary bytes. An authorized initial click was accepted; the actual private page's CDP counters then read pointerdown/up/clicks = 1/1/1.
- I stopped Herdr through this isolated session CLI, observed the socket absent, and relaunched the same named session at that same socket. Replacement Herdr PID 4052995, start_ticks 35262758; 0.9.1/protocol 22 and w1/w1:t1/p1 focus restored.
- Only after the replacement server and socket were live, old-view pointer down and pointer up were sent using the old view/epoch/location/lease and sequence 3/4. Both returned HTTP 400 `browser_view_not_found` (“Browser view is not attached to this owner”). The old page counters remained exactly 1/1/1. Replaying the old frame grant also closed 1008, same refusal reason, with no binary payload.
- Immediately afterward, current target open returned HTTP 200 with new association `c1f3822f2cd62fc58ddd5941` and Chromium incarnation `pid=4053105:start=35262852:receipt=b09bd77504407d57fe5c6003bcfdc5c6f08efd5c9c035b63af100b01e69eda88`. Fresh view `be301b30-94a3-4768-9653-637dcc17021f` received a 6,338-byte frame at sequence 1, took control at lease generation 2, and its click down/up returned HTTP 200 `accepted`; the new page counters were 1/1/1. This supplies the current-association positive after old-view and old-grant refusals.
- Cleanup: current browser close and Herdr session stop returned successfully; the named fixture service was stopped. Gateway PID 4045212 and its group, old Chromium daemon 4052579 and old Chromium/crashpad groups (profile path matched the private root) were stopped only as run-owned resources. Verified no remaining command line referenced `/tmp/web08-rpl-lhbibP`, gateway/fixture ports 45705/36727 refused connections, Herdr socket was absent, and replacement Herdr PID 4052995/gateway PID 4045212 were absent. The private root was removed after verification.


## Cleanup

Fixture service PID 3848281 was stopped through its named owned service. The replacement Herdr session was stopped through `herdr --session web08-f9edd54fdd7b server stop`; gateway PID 3737259 and its helpers were stopped as their run-owned process group, and the remaining old-browser process groups were stopped only after matching their private profile paths. The earlier Herdr PID 3737213 had already been stopped through the same isolated session CLI before replacement. Verified no remaining process command line referenced `/tmp/web08-grant-Z4mTb1`, replacement Herdr PID 3863322 and gateway PID 3737259 were absent, Herdr socket was absent, and gateway/fixture ports 43343/34203 refused connections. Port 5173 was never bound or used. The private root and temporary probe/fixture artifacts were removed after this verification.
