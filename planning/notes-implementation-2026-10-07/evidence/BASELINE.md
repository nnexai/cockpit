# Source-current Notes baseline — B1–B6

Captured 2026-10-07 by NotesBaseline under released Execute736e6c02a7f833131611962abfcec3ebdd1e126119dc9e720c69dbbdf4482526. This is baseline evidence, not final polish acceptance. No product edits, installs, shared dist/target builds, source commits or pushes. The unchanged standard helper's synthetic initial fixture-repository commit was explicitly authorized and executed; the earlier PLAN Q3 omission recipe was **not** used. Only this report and B screenshots persist. All owned live resources have been closed/stopped/deleted; no handoff.

## Actionable results

- **NR21 baseline established:** real CLI depths 0/1/2; rendered padding12/24/36px at1440 and420. Ancestor checkbox centers are280/292/304px wide and32/44/56px narrow. NR48's proposed24px label changes the center reference by4px; baseline uses the original16px checkbox.
- **NR09 literal measure =532px**: real scroller1152px minus310px left and310px right. Fonts settled; current cm-line adds6px left/2px right. Freeze `--notes-measure:532px`, not a font guess.
- **NR43 CONFIRMED:** authoritative known backend alert occludes both Save and Comment centers at1440 and420; Kanban live-status intersects the slot at both widths. See the explicit request-adaptation limitation below.
- **NR49 CONFIRMED / cross-surface:** Notes selected/unselected appearance identical; Supervisor already has selection styling. Complete matching source/runtime inventory is Notes+Supervisor only. Parent notified before any global edit; this worker made none.

## B1 Source/build/runtime identity

Archived the recorded source revision, overlaid all physically copied working-tree `src/app/notes/*` files before coding started, physically copied the standard helper unchanged (not a symlink), reused existing node_modules and read-only host executable symlinks. Non-Notes source, including Supervisor, is the archived revision, not concurrent Supervisor working-tree edits. The single authorized baseline build succeeded: `bun run build` ran tsc noEmit and Vite8.2.2, transformed471 modules and emitted isolated assets. Only the standard bundle-size warning appeared. The real gateway command line contains this snapshot's isolated `--static-dir`, and the browser loaded its exact hashed JS URL.

The reused host passed actual CLI help plus CLI and HTTP Notes catalog/read DTO checks; no host rebuild was needed. The CLI version refused `decision create --text` (accepts stdin/file); the seed uses an actual file instead. Failed preliminary seed invocations wrote no record and were not replayed after success. A preliminary manually supplied HTTP op `catalog` returned notes_usage; corrected actual op `catalog_list` succeeded. These are recorded discovery corrections, not compatibility failures.

```json
{
  "revision": "b538420beac5d2bd843f417559b9bde552bdedee",
  "snapshot": "/tmp/cnotes-baseline-e3z9sox5",
  "overlay_sha256": {
    "AgentAccess.tsx": "a6b836e2e844d494a189847492e936a9fb3e177c953778e45562eeb9198c83ae",
    "Decisions.tsx": "dfbfe49e8616e8b4be6a8e094c77b4c8bad8532e83159f9a9a679c477bc4cf25",
    "Kanban.tsx": "99a7aaca433ba8c78a0d0daec1a220a0b102b01cbde876aa4ff4bf84b37e9422",
    "MarkdownEditor.tsx": "9d7398e4b72a613dcd074c03546a74a1dd4b61c0e2d7fc042ad886641d22154e",
    "NotesView.behavior.test.tsx": "b02f038d6f1382740da47d81634e3ec23d42e4148fc64d48f9544a256657301b",
    "NotesView.tsx": "e0945dfcfe7aff720e1a5d3b4dfab1bb3482c013de5283e0bd947831e59bfe98",
    "RetainedDrafts.tsx": "d61c9ae2e0529f9be1e31f98a91b7b998f74ad2b11417606f4b3ced57d545865",
    "TaskDetail.tsx": "eab0fc5682f904c77f7c42c7ba140ae76eae2e3c78c6148802af6b635e57da90",
    "TodoTitle.tsx": "aa99effb94192c3208dbfa69a6ddd5db811973b8391131156c0bcab5e6fdfd08",
    "boardState.ts": "efb8fe727ada102ad66dc56ebdbed2de9261c6a9403673ad0c0b60015383ff5e",
    "drafts.ts": "eb861f48d6b77a86889cedb962a7bdf2ee54bc0bae7aab1ef5f9f4ccbe04ee70",
    "notes.css": "87e63d721add4a58985da8292880fdb8cd3f008ff2ffb093a5d02043175255ac",
    "notesSensors.ts": "76f0ad41c355650aed547bd5dd159a97ec8b57382d40d32f915c558ebaaef2e9",
    "notesState.test.ts": "a0cbe37c6689089a73ebbd8d4cf0216e1734a151689af90b690712c6690953ed",
    "useNotes.ts": "a79a210305587e0cc5a6aefc2712c1bb773594c2f2e1c57c2b86c1b373623bf2"
  },
  "helper_sha256": "67cefd0a983dc5f83239437dc472e6e9c48bdb479dcaca3b95273215a451dedf",
  "gateway": "/home/nnex/dev/prj/cockpit/target/debug/cockpit",
  "gateway_sha256": "c7705fdd3c42d956f59b3ed896745fad05d7b12de50269649e8fc9fb3d6245a4",
  "registry_sha256": "fdb0c27f0ae652843587a9077706493fa6e8625e74460dd054612f3bbe35a109",
  "fixture": "/tmp/cpol-53ubs82y",
  "ledger": {
    "root": "/tmp/cpol-53ubs82y",
    "session": "polish-53ubs82y",
    "socket": "/tmp/cpol-53ubs82y/config/herdr/sessions/polish-53ubs82y/herdr.sock",
    "protected": [
      "default",
      "pre-existing sessions and gateways"
    ],
    "fixture": "/tmp/cpol-53ubs82y/repositories/sample",
    "fixture_url": "http://127.0.0.1:38917/"
  },
  "gateway_cmdline": "/tmp/cnotes-baseline-e3z9sox5/target/debug/cockpit serve --herdr-session polish-53ubs82y --herdr-socket /tmp/cpol-53ubs82y/config/herdr/sessions/polish-53ubs82y/herdr.sock --config /tmp/cpol-53ubs82y/cockpit.toml --static-dir /tmp/cnotes-baseline-e3z9sox5/dist ",
  "assets": {
    "dist/index.html": "ce9d2c7e48cdd54599afeaced5661c10a9807f35b954f00a74979b156154c369",
    "dist/assets/index-Cwzz5QVU.js": "b0b8f75f0c4e5529643f6d00c634c54bc418b587f4b782d56e6177d87e2b2911",
    "dist/assets/index-r1y2owCy.css": "1c43dcb384507dbd5cb60e886825eed540eec0d29be0bd71a2287bb68d7674e3"
  },
  "uuid": "2ce09e19-bf76-46d2-af52-129f6a262b8c",
  "notes_root": "/tmp/cpol-53ubs82y/data/cockpit/notes",
  "seed_ids": {
    "T1": "wzwso8h2pd",
    "T2": "zfmglsewet",
    "T3": "wwla18ihw0",
    "T4": "ja8rd6tbtx",
    "T5": "kjzy16qdk5",
    "D1": "e58f2d58-5be1-4265-8363-90d5748ec6f7",
    "D2": "6467c7bf-eba5-4b65-a696-9405d6e923c2",
    "D3": "312f0ab3-6f63-4ccd-b161-f43d3a874d97",
    "comment_count": 19
  },
  "seed_hashes": {
    "comments/wzwso8h2pd/10fa23fb-f2e4-4da4-bb7f-88e21b4ddfc2.md": "7de06174baae79cb180ce18a2f0d582152e3fa658255f396e3967e0dfd420207",
    "comments/wzwso8h2pd/1a732a8b-f2e6-464c-8071-489dd3ee3128.md": "0ef47b228b1830c944664a6783c0fe4e4498a73c44a6dbb956f5d99426317944",
    "comments/wzwso8h2pd/265ba5c7-41ef-43a4-b503-320186bc4488.md": "9ad934863ff9ef8b3f363a0576006caa482334c344aa7d7eb844a9d8ba5fbe9f",
    "comments/wzwso8h2pd/318d7338-c635-4825-b045-60fc17181a5d.md": "bbd77ce9060b2916603a49a97f7026949e7a60f307502fe549f2964fa3e58145",
    "comments/wzwso8h2pd/3cd7809e-3ef2-4b51-af51-2ddb8de9346e.md": "29432e609c18cbf33dddc6f568e2fc1fbcea3dcd7320c98dc01c8e72d13fe26c",
    "comments/wzwso8h2pd/3fd4dd0e-9e07-464f-97c0-68b7c5ec6b90.md": "42d178e8d221f090803f6ddbacb0f48b8dd69a78a7efc041303fab059f56e56a",
    "comments/wzwso8h2pd/48edf344-aea6-4a90-9b40-6581e4618b31.md": "a04dbb968b32972dc4ce5178789b423579bdd6702460a440b5be3ce5b75ee566",
    "comments/wzwso8h2pd/4eda50da-a70d-46f7-aa9b-f6652472c50a.md": "b9d3d372bf1104a861422998a840216ceb7c6a076bd721d7d00bd4577a57e797",
    "comments/wzwso8h2pd/5e99399b-91df-4774-9e6b-b3dc2f5e39dd.md": "3a6985e64e1098d1bfa0c80c87063d42a3155965d44e2b4a860c8d94cfc71f68",
    "comments/wzwso8h2pd/7ab9afef-88b2-4cf2-86bb-10eaa8abad81.md": "1c04f3f92277a75fe39a73ed5a503a15eef3332c75a1f567eee70ba90ec9049f",
    "comments/wzwso8h2pd/8ed5558d-e0ba-4865-bd0e-07de2bbdeaff.md": "78439a8f86e1c2ab7a53c6302bae1af16ed833c42e2ff4123bd2596048ec61fa",
    "comments/wzwso8h2pd/91832032-1358-4fae-93f2-5f6fa2f6d156.md": "148353e0bd5703253196337256a7fa60698d020734877e4b7f654933b445c1a6",
    "comments/wzwso8h2pd/a7122cc9-2dea-4cfc-99cd-6b398e30ddec.md": "69cbceea3ad0ae4c470d261353a0e509237dc4190a3bf75558b7d0e33bd672ba",
    "comments/wzwso8h2pd/a8ca850f-832b-4073-94eb-20e5d0bf4a63.md": "ae6ab27e8ce05ebc6cb726f898b11655bf043948f5d49dabc377a92055bcb1f2",
    "comments/wzwso8h2pd/c4446b0c-b10b-4fb3-9de1-dc6bbc7d0f88.md": "846e96df5fadfb2fe81917d22e12eadf570c060e9aa80485e86adc63181450e5",
    "comments/wzwso8h2pd/e554ae39-e612-405e-9569-e5da6fdda6c9.md": "d30da6b5ca2077e82ce3b93fc1fe9d59ff827703f3de0676c040d0024416e4f7",
    "comments/wzwso8h2pd/e572aeb8-3464-4303-a64e-8c5de4ee3593.md": "06193004236602b2d282891658435b5edc51da9bae675f649a3a503cb4e4d5d1",
    "comments/wzwso8h2pd/e8e19583-04dd-4d8e-bd52-9f6b99b0e1de.md": "eb0e9efa133d745906785d15d3c9f0aaa76a205a9b9fabc135ac4ef801103923",
    "comments/wzwso8h2pd/eefa70ac-30d4-4e2b-a3b3-8f1142acb389.md": "c3cdc9e9c42e87d0f789b868e890c89d9986c87e80f32c65cc23be7ebcb8bfc8",
    "decisions/312f0ab3-6f63-4ccd-b161-f43d3a874d97.md": "ee69bcbd1892b6328ade9f3a5ad2f32e59aa06bf07e34cd1b4429e0759cd2b96",
    "decisions/6467c7bf-eba5-4b65-a696-9405d6e923c2.md": "736b0bda522ee09e7c0166af00f4dee68779fd9a42d8e712afdde3c3fa238f5e",
    "decisions/e58f2d58-5be1-4265-8363-90d5748ec6f7.md": "f5991cef5da8d8ee27e144dac2e5e6d6c6012f041f01e942311026c529b355d1",
    "scratchpad.md": "77b8d429b065d186d6bbf436d8af228c5e39fbf9419535fe67fba8d5fbda1adb",
    "todos.md": "5c0cad5e53e380372e8e6381b6a605335ff7086deb2b3fa6a50ea467da7d54cb"
  },
  "S2": {
    "id": "polish-proof",
    "result": {
      "type": "workspace_created",
      "workspace": {
        "workspace_id": "w2",
        "number": 2,
        "label": "Polish S2",
        "focused": false,
        "pane_count": 1,
        "tab_count": 1,
        "active_tab_id": "w2:t1",
        "agent_status": "unknown"
      },
      "tab": {
        "tab_id": "w2:t1",
        "workspace_id": "w2",
        "number": 1,
        "label": "1",
        "focused": false,
        "pane_count": 1,
        "agent_status": "unknown"
      },
      "root_pane": {
        "pane_id": "w2:p1",
        "terminal_id": "term_65d386a57f1252",
        "workspace_id": "w2",
        "tab_id": "w2:t1",
        "focused": false,
        "cwd": "/tmp/cpol-53ubs82y/repositories/sample",
        "foreground_cwd": "/tmp/cpol-53ubs82y/repositories/sample",
        "agent_status": "unknown",
        "scroll": {
          "offset_from_bottom": 0,
          "max_offset_from_bottom": 0,
          "viewport_rows": 40
        },
        "revision": 0
      }
    }
  },
  "S3": {
    "id": "polish-proof",
    "result": {
      "type": "workspace_created",
      "workspace": {
        "workspace_id": "w3",
        "number": 3,
        "label": "Polish S3",
        "focused": false,
        "pane_count": 1,
        "tab_count": 1,
        "active_tab_id": "w3:t1",
        "agent_status": "unknown"
      },
      "tab": {
        "tab_id": "w3:t1",
        "workspace_id": "w3",
        "number": 1,
        "label": "1",
        "focused": false,
        "pane_count": 1,
        "agent_status": "unknown"
      },
      "root_pane": {
        "pane_id": "w3:p1",
        "terminal_id": "term_65d386a5956543",
        "workspace_id": "w3",
        "tab_id": "w3:t1",
        "focused": false,
        "cwd": "/tmp/cpol-53ubs82y/repositories/sample",
        "foreground_cwd": "/tmp/cpol-53ubs82y/repositories/sample",
        "agent_status": "unknown",
        "scroll": {
          "offset_from_bottom": 0,
          "max_offset_from_bottom": 0,
          "viewport_rows": 40
        },
        "revision": 0
      }
    }
  },
  "processes": {
    "herdr": {
      "pid": "3948184",
      "cmdline": "/home/linuxbrew/.linuxbrew/bin/herdr --session polish-53ubs82y server "
    },
    "gateway": {
      "pid": "3948266",
      "cmdline": "/tmp/cnotes-baseline-e3z9sox5/target/debug/cockpit serve --herdr-session polish-53ubs82y --herdr-socket /tmp/cpol-53ubs82y/config/herdr/sessions/polish-53ubs82y/herdr.sock --config /tmp/cpol-53ubs82y/cockpit.toml --static-dir /tmp/cnotes-baseline-e3z9sox5/dist "
    },
    "fixture": {
      "pid": "3948112",
      "cmdline": "/home/linuxbrew/.linuxbrew/opt/python@3.14/bin/python3.14 -u -c from http.server import ThreadingHTTPServer,SimpleHTTPRequestHandler; from functools import partial; import sys; server=ThreadingHTTPServer(('127.0.0.1',0),partial(SimpleHTTPRequestHandler,directory=sys.argv[1])); print(server.server_port,flush=True); server.serve_forever() /tmp/cpol-53ubs82y/www "
    }
  },
  "workspace": {
    "id": "polish-proof",
    "result": {
      "type": "workspace_created",
      "workspace": {
        "workspace_id": "w1",
        "number": 1,
        "label": "Polish sample",
        "focused": true,
        "pane_count": 1,
        "tab_count": 1,
        "active_tab_id": "w1:t1",
        "agent_status": "unknown"
      },
      "tab": {
        "tab_id": "w1:t1",
        "workspace_id": "w1",
        "number": 1,
        "label": "1",
        "focused": true,
        "pane_count": 1,
        "agent_status": "unknown"
      },
      "root_pane": {
        "pane_id": "w1:p1",
        "terminal_id": "term_65d38679908961",
        "workspace_id": "w1",
        "tab_id": "w1:t1",
        "focused": true,
        "cwd": "/tmp/cpol-53ubs82y/repositories/sample",
        "foreground_cwd": "/tmp/cpol-53ubs82y/repositories/sample",
        "agent_status": "unknown",
        "scroll": {
          "offset_from_bottom": 0,
          "max_offset_from_bottom": 0,
          "viewport_rows": 40
        },
        "revision": 0
      }
    }
  },
  "gateway_log": "listening http://127.0.0.1:40409\n",
  "cli_catalog": {
    "exit": 0,
    "body": {
      "notes_id": null,
      "changed": false,
      "result": {
        "kind": "catalog",
        "entries": [
          {
            "notes_id": "2ce09e19-bf76-46d2-af52-129f6a262b8c",
            "label": "Polish sample",
            "created": "2026-10-07T04:18:28.23043245Z",
            "bound": true
          }
        ]
      }
    }
  },
  "cleanup": {
    "browser": "notes-baseline",
    "targetId": "80A48AA1D26C525B23092FE2AF2A78AB",
    "browser_closed": true,
    "helper_stop": "Stopped matching fixture processes; evidence retained at /tmp/cpol-53ubs82y",
    "registry_bytes_equal": true,
    "content_hashes_equal": true,
    "fixture_removed": true,
    "snapshot_removed": true,
    "live_handoff": false
  },
  "screenshots": {
    "B3-nested-1440.png": {
      "sha256": "d26abf9298d5c6285dcd077f55af1fa7cd3c0ab90b0dba4ae5b4a1b7c70c74fe",
      "dimensions": [
        1800,
        1250
      ]
    },
    "B2-todos-1440.png": {
      "sha256": "341400b1c66927f209874b1ad9d33a5effe9dd18fc512c967c50293760ec54ee",
      "dimensions": [
        1800,
        1250
      ]
    },
    "B2-todos-900.png": {
      "sha256": "954b605564e3e1ef21685a0ca376b185f43bb2bb65bedfdd178f6f2c744e2cc0",
      "dimensions": [
        1125,
        1125
      ]
    },
    "B2-todos-420.png": {
      "sha256": "11daf73f1868599f4cf1b4a6ad56cb1bfc0823d83745aef16b424c42aa0e6af2",
      "dimensions": [
        525,
        1125
      ]
    },
    "B2-todos-1100.png": {
      "sha256": "fec6dcb483bdf02afd18a1bbfe2cf39dc8ad776f23c5715ebb7ca3fca5f568e8",
      "dimensions": [
        1375,
        1125
      ]
    },
    "B4-known-save-1440.png": {
      "sha256": "16b853bbb6c3d40de8489b2e4dc33421b7d60a7af50fcb94b7f7eed58fe49a5d",
      "dimensions": [
        1800,
        1250
      ]
    },
    "B4-known-comment-1440.png": {
      "sha256": "08fd40b8c79bc5352fdf00cf5dc978dfbe07bb2a022d2013ed21eb72ba512efa",
      "dimensions": [
        1800,
        1250
      ]
    },
    "B4-known-save-420.png": {
      "sha256": "518f26eb2da5c3543a29e04ec1bf860ac9994d2c57d1b7388e6fdf1a6e6fdcd8",
      "dimensions": [
        525,
        1125
      ]
    },
    "B4-known-comment-420.png": {
      "sha256": "1035d2867a55f4b5d6512051f2daf017b9524639830da35afe11690e472735d3",
      "dimensions": [
        525,
        1125
      ]
    },
    "B5-supervisor-pressed.png": {
      "sha256": "2b08acfccdeb3f1fccdc1cdc13acd722549c7b932862af0b67e51ba5e9bb8c4a",
      "dimensions": [
        1800,
        1250
      ]
    },
    "B5-notes-pressed.png": {
      "sha256": "a62c43b6e4eb5e349d2ea434b3a307912e1d059a14bb30c6b5119a2bff46c97f",
      "dimensions": [
        1800,
        1250
      ]
    },
    "B6-card-1440.png": {
      "sha256": "b09f97a4f0319b0cc540168b5b8a4372acab23f9344d658c0b8e1c9f8de890ba",
      "dimensions": [
        1800,
        1250
      ]
    },
    "B6-short-decision-1440.png": {
      "sha256": "a247f68a64acfae255dea04a842c67f78624ad3d3d1532df3292318d5ef5dc48",
      "dimensions": [
        1800,
        1250
      ]
    },
    "B6-picker-S3.png": {
      "sha256": "67b946a1e9a28301e27b9dcae9126db5302caafbb8a51bca0d379fa06bb020e0",
      "dimensions": [
        1800,
        1250
      ]
    },
    "B6-source-1440.png": {
      "sha256": "87de60aa2399f1fdbebaf1f382b05d175cd788fc1c6bf9958ca894f0d951c7ea",
      "dimensions": [
        1800,
        1250
      ]
    },
    "B5-notes-unpressed.png": {
      "sha256": "9109a7e718fae015548ed7e4e683943609fd37d0578d9cc3f11ec7a32598d17e",
      "dimensions": [
        1800,
        1250
      ]
    },
    "B3-nested-420.png": {
      "sha256": "ab50770993ef47b3bd7c7abb9d58adf94d662d1c26ff4a8740be00e472d647e3",
      "dimensions": [
        525,
        1125
      ]
    }
  },
  "runtime_ledger_path": "/tmp/cpol-53ubs82y/runtime.json",
  "build": "bun run build; tsc --noEmit and Vite8.2.2; exit0,471 modules,893ms; one baseline build only",
  "static_dir": "/tmp/cnotes-baseline-e3z9sox5/dist",
  "reused_dependencies": "/home/nnex/dev/prj/cockpit/node_modules",
  "helper_unchanged": true,
  "cli_reads": [
    {
      "args": [
        "scratchpad",
        "read"
      ],
      "exit": 0,
      "notes_id": "2ce09e19-bf76-46d2-af52-129f6a262b8c",
      "changed": false,
      "kind": "scratchpad"
    },
    {
      "args": [
        "todo",
        "list"
      ],
      "exit": 0,
      "notes_id": "2ce09e19-bf76-46d2-af52-129f6a262b8c",
      "changed": false,
      "kind": "todos"
    },
    {
      "args": [
        "kanban",
        "list"
      ],
      "exit": 0,
      "notes_id": "2ce09e19-bf76-46d2-af52-129f6a262b8c",
      "changed": false,
      "kind": "board"
    },
    {
      "args": [
        "decision",
        "list",
        "--status",
        "all"
      ],
      "exit": 0,
      "notes_id": "2ce09e19-bf76-46d2-af52-129f6a262b8c",
      "changed": false,
      "kind": "decisions"
    },
    {
      "args": [
        "scratchpad",
        "read"
      ],
      "exit": 0,
      "notes_id": "2ce09e19-bf76-46d2-af52-129f6a262b8c",
      "changed": false,
      "kind": "scratchpad"
    },
    {
      "args": [
        "todo",
        "list"
      ],
      "exit": 0,
      "notes_id": "2ce09e19-bf76-46d2-af52-129f6a262b8c",
      "changed": false,
      "kind": "todos"
    }
  ],
  "required_help_exit_codes": {
    "todo list --help": 0,
    "kanban list --help": 0,
    "decision create --help": 0,
    "decision list --help": 0,
    "comment list --help": 0
  },
  "browser": {
    "name": "notes-baseline",
    "targetId": "80A48AA1D26C525B23092FE2AF2A78AB",
    "backend": "managed headless Chromium",
    "origin": "http://127.0.0.1:40409",
    "deviceScaleFactor": 1.25
  },
  "loaded_script": "http://127.0.0.1:40409/assets/index-Cwzz5QVU.js"
}
```
### Actual HTTP compatibility

All pinned reads below returned HTTP200, exact UI UUID, changed:false, and the expected result kinds. Catalog is root-targeted, notes_id:null, changed:false.

```json
{
  "reads": [
    {
      "operation": {
        "op": "scratchpad_read"
      },
      "status": 200,
      "notes_id": "2ce09e19-bf76-46d2-af52-129f6a262b8c",
      "changed": false,
      "result": "scratchpad"
    },
    {
      "operation": {
        "op": "todo_list",
        "filter": "all"
      },
      "status": 200,
      "notes_id": "2ce09e19-bf76-46d2-af52-129f6a262b8c",
      "changed": false,
      "result": "todos"
    },
    {
      "operation": {
        "op": "kanban_list"
      },
      "status": 200,
      "notes_id": "2ce09e19-bf76-46d2-af52-129f6a262b8c",
      "changed": false,
      "result": "board"
    },
    {
      "operation": {
        "op": "decision_list",
        "status": "all",
        "query": null
      },
      "status": 200,
      "notes_id": "2ce09e19-bf76-46d2-af52-129f6a262b8c",
      "changed": false,
      "result": "decisions"
    },
    {
      "operation": {
        "op": "comment_list",
        "todo_id": "wzwso8h2pd"
      },
      "status": 200,
      "notes_id": "2ce09e19-bf76-46d2-af52-129f6a262b8c",
      "changed": false,
      "result": "comments"
    }
  ],
  "catalog": {
    "status": 200,
    "body": {
      "notes_id": null,
      "changed": false,
      "result": {
        "kind": "catalog",
        "entries": [
          {
            "notes_id": "2ce09e19-bf76-46d2-af52-129f6a262b8c",
            "label": "Polish sample",
            "created": "2026-10-07T04:18:28.23043245Z",
            "bound": true
          }
        ]
      }
    }
  }
}
```
## B2 Viewport versus actual Notes container

| Viewport | Notes width | Actual band |
|---|---:|---|
|1440×1000|1200px|≥900|
|900×900|660px|521–719|
|420×900|420px|≤520|
|1100×900 (explicit band check)|860px|720–899|

Default sidebar remains240px at wide/mid widths; narrow sidebar collapses. Browser DPR1.25, so PNG physical pixel dimensions are1.25×CSS viewport. Captures: B2-todos-1440.png, B2-todos-900.png, B2-todos-420.png, B2-todos-1100.png. No mock or static preview was substituted.

```json
[
  {
    "viewport": 1440,
    "notes": 1200,
    "notesRect": {
      "x": 240,
      "y": 41,
      "width": 1200,
      "height": 931,
      "top": 41,
      "right": 1440,
      "bottom": 972,
      "left": 240
    }
  },
  {
    "viewport": 900,
    "notes": 660,
    "notesRect": {
      "x": 240,
      "y": 41,
      "width": 660,
      "height": 831,
      "top": 41,
      "right": 900,
      "bottom": 872,
      "left": 240
    }
  },
  {
    "viewport": 420,
    "notes": 420,
    "notesRect": {
      "x": 0,
      "y": 41,
      "width": 420,
      "height": 831,
      "top": 41,
      "right": 420,
      "bottom": 872,
      "left": 0
    }
  },
  {
    "viewport": 1100,
    "notes": 860,
    "notesRect": {
      "x": 240,
      "y": 41,
      "width": 860,
      "height": 831,
      "top": 41,
      "right": 1100,
      "bottom": 872,
      "left": 240
    }
  }
]
```
## B3 Nested rows (NR21)

Captures: B3-nested-1440.png and B3-nested-420.png. Original no-ID rows use their complete returned `ref` as data-todo-id; opening their detail was avoided, so no adoption occurred. CLI depth is observed, not inferred. Pinned rows remain in canonical source order.

```json
[
  {
    "data-todo-id": "wzwso8h2pd",
    "title": "Adopted board task",
    "CLI_depth": 0,
    "wide_padding": "12px",
    "narrow_padding": "12px",
    "wide_checkbox_center": 280,
    "narrow_checkbox_center": 32
  },
  {
    "data-todo-id": "zfmglsewet",
    "title": "Doing task",
    "CLI_depth": 0,
    "wide_padding": "12px",
    "narrow_padding": "12px",
    "wide_checkbox_center": 280,
    "narrow_checkbox_center": 32
  },
  {
    "data-todo-id": "wwla18ihw0",
    "title": "Plain todo",
    "CLI_depth": 0,
    "wide_padding": "12px",
    "narrow_padding": "12px",
    "wide_checkbox_center": 280,
    "narrow_checkbox_center": 32
  },
  {
    "data-todo-id": "ja8rd6tbtx",
    "title": "Completed task",
    "CLI_depth": 0,
    "wide_padding": "12px",
    "narrow_padding": "12px",
    "wide_checkbox_center": 280,
    "narrow_checkbox_center": 32
  },
  {
    "data-todo-id": "kjzy16qdk5",
    "title": "One-line no-comment board task",
    "CLI_depth": 0,
    "wide_padding": "12px",
    "narrow_padding": "12px",
    "wide_checkbox_center": 280,
    "narrow_checkbox_center": 32
  },
  {
    "data-todo-id": "L6@sha256:5c0cad5e53e380372e8e6381b6a605335ff7086deb2b3fa6a50ea467da7d54cb",
    "title": "Hand-written parent",
    "CLI_depth": 0,
    "wide_padding": "12px",
    "narrow_padding": "12px",
    "wide_checkbox_center": 280,
    "narrow_checkbox_center": 32
  },
  {
    "data-todo-id": "L7@sha256:5c0cad5e53e380372e8e6381b6a605335ff7086deb2b3fa6a50ea467da7d54cb",
    "title": "Nested child one",
    "CLI_depth": 1,
    "wide_padding": "24px",
    "narrow_padding": "24px",
    "wide_checkbox_center": 292,
    "narrow_checkbox_center": 44
  },
  {
    "data-todo-id": "L8@sha256:5c0cad5e53e380372e8e6381b6a605335ff7086deb2b3fa6a50ea467da7d54cb",
    "title": "Nested child two",
    "CLI_depth": 2,
    "wide_padding": "36px",
    "narrow_padding": "36px",
    "wide_checkbox_center": 304,
    "narrow_checkbox_center": 56
  },
  {
    "data-todo-id": "L9@sha256:5c0cad5e53e380372e8e6381b6a605335ff7086deb2b3fa6a50ea467da7d54cb",
    "title": "Hand-written unadopted task with a deliberately long title that wraps across two lines in a narrow lane",
    "CLI_depth": 0,
    "wide_padding": "12px",
    "narrow_padding": "12px",
    "wide_checkbox_center": 280,
    "narrow_checkbox_center": 32
  }
]
```
## B4 Actual error, hit tests and byte preservation

### Necessary recipe correction — no product mutation

The literal UI date `2026-13-45` is rejected by source-current `notesProtocol.ts:71–77` before any HTTP request. That local malformed-request failure is classified as notes_outcome_unknown; it is **not** a known backend error. The initial wide Save probe nevertheless hit the Check button inside the slot. After Check → explicit ack, an already-adopted T1 detail was opened, then the known-error scenario used this clearly separated setup:

- **F setup:** enter valid UI date2026-10-01, intercept exactly its outgoing decision_create request in the owned browser, replace only `operation.decided` with2026-13-45 and forward to the real gateway. No product or backend code was altered. Interception was removed immediately.
- **R response:** the real backend returned HTTP400 `notes_invalid_input`; its response was delivered unchanged to the app. This produced the actual known alert. An independent direct HTTP invalid-date call returned the same authoritative error. Both refusal paths left the exact content hashes unchanged.

This establishes real known-alert geometry but does not claim the original literal UI recipe reaches the backend. Main was notified with the correction; final known-error evidence should preserve this classification or use another actual authoritative error state.

```json
{
  "direct_backend": {
    "status": 400,
    "body": "{\"code\":\"notes_invalid_input\",\"message\":\"Decided must be a date or RFC3339 timestamp\"}"
  },
  "adapted_UI_backend": [
    {
      "original": "{\"target\":{\"kind\":\"notes\",\"notes_id\":\"2ce09e19-bf76-46d2-af52-129f6a262b8c\"},\"operation\":{\"op\":\"decision_create\",\"title\":\"Probe\",\"body\":\"\",\"decided\":\"2026-10-01\"}}",
      "adapted": {
        "target": {
          "kind": "notes",
          "notes_id": "2ce09e19-bf76-46d2-af52-129f6a262b8c"
        },
        "operation": {
          "op": "decision_create",
          "title": "Probe",
          "body": "",
          "decided": "2026-13-45"
        }
      }
    },
    {
      "status": 400,
      "body": "{\"code\":\"notes_invalid_input\",\"message\":\"Decided must be a date or RFC3339 timestamp\"}"
    }
  ]
}
```
### Known-alert occlusion

Save had a one-character scratchpad draft; Comment had a one-character composer draft on adopted T1, so no adoption cleared the error. At1440 both centers resolved to Check saved state, not their own control. At420 both centers resolved to the alert container. Their controls were enabled under the known error. The wide Kanban probe retained detail; the narrow Kanban probe closed detail before measuring the visible board. Captures: B4-known-save-1440.png, B4-known-comment-1440.png, B4-known-save-420.png, B4-known-comment-420.png.

```json
{
  "save1440": {
    "selector": ".notes-scratchpad .notes-primary",
    "rect": {
      "x": 1307.484375,
      "y": 899,
      "width": 108.515625,
      "height": 28,
      "top": 899,
      "right": 1416,
      "bottom": 927,
      "left": 1307.484375
    },
    "hit": "<button type=\"button\">Check saved state</button>",
    "owned": false,
    "slot": true,
    "slotRect": {
      "x": 240,
      "y": 896,
      "width": 1200,
      "height": 41,
      "top": 896,
      "right": 1440,
      "bottom": 937,
      "left": 240
    },
    "active": "<button type=\"button\" role=\"tab\" id=\"f-scratchpad\" aria-label=\"Scratchpad\" aria-selected=\"true\" aria-controls=\"notes-scratchpad-panel\" tabindex=\"0\"><svg class=\"ui-icon\" viewBox=\"0 0 24 24\" aria-hidden=\"true\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.5\" stroke-linecap=\"round\" stroke-linejoin="
  },
  "comment1440": {
    "selector": "#postCommentBtn",
    "rect": {
      "x": 1331.25,
      "y": 897,
      "width": 94.75,
      "height": 28,
      "top": 897,
      "right": 1426,
      "bottom": 925,
      "left": 1331.25
    },
    "hit": "<button type=\"button\">Check saved state</button>",
    "owned": false,
    "slot": true,
    "slotRect": {
      "x": 240,
      "y": 896,
      "width": 1200,
      "height": 41,
      "top": 896,
      "right": 1440,
      "bottom": 937,
      "left": 240
    },
    "active": "<div spellcheck=\"false\" autocorrect=\"off\" autocapitalize=\"off\" writingsuggestions=\"false\" translate=\"no\" contenteditable=\"true\" style=\"tab-size: 4;\" class=\"cm-content cm-lineWrapping\" role=\"textbox\" aria-multiline=\"true\" aria-label=\"New comment Markdown\" aria-placeholder=\"Add a comment\u2026\" data-langua"
  },
  "board1440": {
    "live": {
      "x": 240,
      "y": 919,
      "width": 840,
      "height": 18,
      "top": 919,
      "right": 1080,
      "bottom": 937,
      "left": 240
    },
    "slot": {
      "x": 240,
      "y": 896,
      "width": 1200,
      "height": 41,
      "top": 896,
      "right": 1440,
      "bottom": 937,
      "left": 240
    },
    "intersects": true
  },
  "save420": {
    "selector": ".notes-scratchpad .notes-primary",
    "rect": {
      "x": 293.484375,
      "y": 799,
      "width": 108.515625,
      "height": 28,
      "top": 799,
      "right": 402,
      "bottom": 827,
      "left": 293.484375
    },
    "hit": "<div class=\"notes-error\" role=\"alert\"><svg class=\"ui-icon\" viewBox=\"0 0 24 24\" aria-hidden=\"true\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.5\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><path d=\"M12 11v6m0-10v1M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0\"></path></svg><span>Decided must be a date or RFC3339 timestamp</span><code>notes_invalid_input</code><button type=\"button\">Check saved state</button></div>",
    "owned": false,
    "slot": true,
    "slotRect": {
      "x": 0,
      "y": 753.21875,
      "width": 420,
      "height": 83.78125,
      "top": 753.21875,
      "right": 420,
      "bottom": 837,
      "left": 0
    },
    "active": "<button type=\"button\" role=\"tab\" id=\"f-scratchpad\" aria-label=\"Scratchpad\" aria-selected=\"true\" aria-controls=\"notes-scratchpad-panel\" tabindex=\"0\"><svg class=\"ui-icon\" viewBox=\"0 0 24 24\" aria-hidden=\"true\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.5\" stroke-linecap=\"round\" stroke-linejoin="
  },
  "comment420": {
    "selector": "#postCommentBtn",
    "rect": {
      "x": 311.25,
      "y": 797,
      "width": 94.75,
      "height": 28,
      "top": 797,
      "right": 406,
      "bottom": 825,
      "left": 311.25
    },
    "hit": "<div class=\"notes-error\" role=\"alert\"><svg class=\"ui-icon\" viewBox=\"0 0 24 24\" aria-hidden=\"true\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.5\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><path d=\"M12 11v6m0-10v1M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0\"></path></svg><span>Decided must be a date or RFC3339 timestamp</span><code>notes_invalid_input</code><button type=\"button\">Check saved state</button></div>",
    "owned": false,
    "slot": true,
    "slotRect": {
      "x": 0,
      "y": 753.21875,
      "width": 420,
      "height": 83.78125,
      "top": 753.21875,
      "right": 420,
      "bottom": 837,
      "left": 0
    },
    "active": "<h3 id=\"notes-task-detail-title\" tabindex=\"-1\">Task details</h3>"
  },
  "board420": {
    "live": {
      "x": 0,
      "y": 819,
      "width": 420,
      "height": 18,
      "top": 819,
      "right": 420,
      "bottom": 837,
      "left": 0
    },
    "slot": {
      "x": 0,
      "y": 753.21875,
      "width": 420,
      "height": 83.78125,
      "top": 753.21875,
      "right": 420,
      "bottom": 837,
      "left": 0
    },
    "intersects": true
  }
}
```
### Check/discard evidence

Known Check saved state cleared the alert. Cancel (discard draft) discarded the New decision draft. To discard the two unsaved fixture-only editor drafts without saving, removed only `cockpit.notes.drafts.v1:2ce09e19-bf76-46d2-af52-129f6a262b8c` from the owned browser's localStorage and reloaded, clearing its in-memory drafts. Reopening Notes had0 `.notes-error` elements. This is disposable draft cleanup, not product persistence evidence. The unknown branch earlier required Check and explicit ack before writes were enabled; no denied mutation was replayed.

Exact post-seed and final content hash dictionaries are equal (all24 content files;19 comments,3 decisions,scratchpad,todos). Hash checkpoints were captured after seed and after the complete probe sequence, **not** separately before/after each B step; therefore this proves net unchanged content across baseline, not per-step transient history. No baseline UI content mutation was submitted successfully. User Herdr plugin registry bytes were captured read-only before/after and were exactly equal. Its exact SHA256 is in B1.

```json
{
  "H_post_seed": {
    "comments/wzwso8h2pd/10fa23fb-f2e4-4da4-bb7f-88e21b4ddfc2.md": "7de06174baae79cb180ce18a2f0d582152e3fa658255f396e3967e0dfd420207",
    "comments/wzwso8h2pd/1a732a8b-f2e6-464c-8071-489dd3ee3128.md": "0ef47b228b1830c944664a6783c0fe4e4498a73c44a6dbb956f5d99426317944",
    "comments/wzwso8h2pd/265ba5c7-41ef-43a4-b503-320186bc4488.md": "9ad934863ff9ef8b3f363a0576006caa482334c344aa7d7eb844a9d8ba5fbe9f",
    "comments/wzwso8h2pd/318d7338-c635-4825-b045-60fc17181a5d.md": "bbd77ce9060b2916603a49a97f7026949e7a60f307502fe549f2964fa3e58145",
    "comments/wzwso8h2pd/3cd7809e-3ef2-4b51-af51-2ddb8de9346e.md": "29432e609c18cbf33dddc6f568e2fc1fbcea3dcd7320c98dc01c8e72d13fe26c",
    "comments/wzwso8h2pd/3fd4dd0e-9e07-464f-97c0-68b7c5ec6b90.md": "42d178e8d221f090803f6ddbacb0f48b8dd69a78a7efc041303fab059f56e56a",
    "comments/wzwso8h2pd/48edf344-aea6-4a90-9b40-6581e4618b31.md": "a04dbb968b32972dc4ce5178789b423579bdd6702460a440b5be3ce5b75ee566",
    "comments/wzwso8h2pd/4eda50da-a70d-46f7-aa9b-f6652472c50a.md": "b9d3d372bf1104a861422998a840216ceb7c6a076bd721d7d00bd4577a57e797",
    "comments/wzwso8h2pd/5e99399b-91df-4774-9e6b-b3dc2f5e39dd.md": "3a6985e64e1098d1bfa0c80c87063d42a3155965d44e2b4a860c8d94cfc71f68",
    "comments/wzwso8h2pd/7ab9afef-88b2-4cf2-86bb-10eaa8abad81.md": "1c04f3f92277a75fe39a73ed5a503a15eef3332c75a1f567eee70ba90ec9049f",
    "comments/wzwso8h2pd/8ed5558d-e0ba-4865-bd0e-07de2bbdeaff.md": "78439a8f86e1c2ab7a53c6302bae1af16ed833c42e2ff4123bd2596048ec61fa",
    "comments/wzwso8h2pd/91832032-1358-4fae-93f2-5f6fa2f6d156.md": "148353e0bd5703253196337256a7fa60698d020734877e4b7f654933b445c1a6",
    "comments/wzwso8h2pd/a7122cc9-2dea-4cfc-99cd-6b398e30ddec.md": "69cbceea3ad0ae4c470d261353a0e509237dc4190a3bf75558b7d0e33bd672ba",
    "comments/wzwso8h2pd/a8ca850f-832b-4073-94eb-20e5d0bf4a63.md": "ae6ab27e8ce05ebc6cb726f898b11655bf043948f5d49dabc377a92055bcb1f2",
    "comments/wzwso8h2pd/c4446b0c-b10b-4fb3-9de1-dc6bbc7d0f88.md": "846e96df5fadfb2fe81917d22e12eadf570c060e9aa80485e86adc63181450e5",
    "comments/wzwso8h2pd/e554ae39-e612-405e-9569-e5da6fdda6c9.md": "d30da6b5ca2077e82ce3b93fc1fe9d59ff827703f3de0676c040d0024416e4f7",
    "comments/wzwso8h2pd/e572aeb8-3464-4303-a64e-8c5de4ee3593.md": "06193004236602b2d282891658435b5edc51da9bae675f649a3a503cb4e4d5d1",
    "comments/wzwso8h2pd/e8e19583-04dd-4d8e-bd52-9f6b99b0e1de.md": "eb0e9efa133d745906785d15d3c9f0aaa76a205a9b9fabc135ac4ef801103923",
    "comments/wzwso8h2pd/eefa70ac-30d4-4e2b-a3b3-8f1142acb389.md": "c3cdc9e9c42e87d0f789b868e890c89d9986c87e80f32c65cc23be7ebcb8bfc8",
    "decisions/312f0ab3-6f63-4ccd-b161-f43d3a874d97.md": "ee69bcbd1892b6328ade9f3a5ad2f32e59aa06bf07e34cd1b4429e0759cd2b96",
    "decisions/6467c7bf-eba5-4b65-a696-9405d6e923c2.md": "736b0bda522ee09e7c0166af00f4dee68779fd9a42d8e712afdde3c3fa238f5e",
    "decisions/e58f2d58-5be1-4265-8363-90d5748ec6f7.md": "f5991cef5da8d8ee27e144dac2e5e6d6c6012f041f01e942311026c529b355d1",
    "scratchpad.md": "77b8d429b065d186d6bbf436d8af228c5e39fbf9419535fe67fba8d5fbda1adb",
    "todos.md": "5c0cad5e53e380372e8e6381b6a605335ff7086deb2b3fa6a50ea467da7d54cb"
  },
  "H_final": {
    "comments/wzwso8h2pd/10fa23fb-f2e4-4da4-bb7f-88e21b4ddfc2.md": "7de06174baae79cb180ce18a2f0d582152e3fa658255f396e3967e0dfd420207",
    "comments/wzwso8h2pd/1a732a8b-f2e6-464c-8071-489dd3ee3128.md": "0ef47b228b1830c944664a6783c0fe4e4498a73c44a6dbb956f5d99426317944",
    "comments/wzwso8h2pd/265ba5c7-41ef-43a4-b503-320186bc4488.md": "9ad934863ff9ef8b3f363a0576006caa482334c344aa7d7eb844a9d8ba5fbe9f",
    "comments/wzwso8h2pd/318d7338-c635-4825-b045-60fc17181a5d.md": "bbd77ce9060b2916603a49a97f7026949e7a60f307502fe549f2964fa3e58145",
    "comments/wzwso8h2pd/3cd7809e-3ef2-4b51-af51-2ddb8de9346e.md": "29432e609c18cbf33dddc6f568e2fc1fbcea3dcd7320c98dc01c8e72d13fe26c",
    "comments/wzwso8h2pd/3fd4dd0e-9e07-464f-97c0-68b7c5ec6b90.md": "42d178e8d221f090803f6ddbacb0f48b8dd69a78a7efc041303fab059f56e56a",
    "comments/wzwso8h2pd/48edf344-aea6-4a90-9b40-6581e4618b31.md": "a04dbb968b32972dc4ce5178789b423579bdd6702460a440b5be3ce5b75ee566",
    "comments/wzwso8h2pd/4eda50da-a70d-46f7-aa9b-f6652472c50a.md": "b9d3d372bf1104a861422998a840216ceb7c6a076bd721d7d00bd4577a57e797",
    "comments/wzwso8h2pd/5e99399b-91df-4774-9e6b-b3dc2f5e39dd.md": "3a6985e64e1098d1bfa0c80c87063d42a3155965d44e2b4a860c8d94cfc71f68",
    "comments/wzwso8h2pd/7ab9afef-88b2-4cf2-86bb-10eaa8abad81.md": "1c04f3f92277a75fe39a73ed5a503a15eef3332c75a1f567eee70ba90ec9049f",
    "comments/wzwso8h2pd/8ed5558d-e0ba-4865-bd0e-07de2bbdeaff.md": "78439a8f86e1c2ab7a53c6302bae1af16ed833c42e2ff4123bd2596048ec61fa",
    "comments/wzwso8h2pd/91832032-1358-4fae-93f2-5f6fa2f6d156.md": "148353e0bd5703253196337256a7fa60698d020734877e4b7f654933b445c1a6",
    "comments/wzwso8h2pd/a7122cc9-2dea-4cfc-99cd-6b398e30ddec.md": "69cbceea3ad0ae4c470d261353a0e509237dc4190a3bf75558b7d0e33bd672ba",
    "comments/wzwso8h2pd/a8ca850f-832b-4073-94eb-20e5d0bf4a63.md": "ae6ab27e8ce05ebc6cb726f898b11655bf043948f5d49dabc377a92055bcb1f2",
    "comments/wzwso8h2pd/c4446b0c-b10b-4fb3-9de1-dc6bbc7d0f88.md": "846e96df5fadfb2fe81917d22e12eadf570c060e9aa80485e86adc63181450e5",
    "comments/wzwso8h2pd/e554ae39-e612-405e-9569-e5da6fdda6c9.md": "d30da6b5ca2077e82ce3b93fc1fe9d59ff827703f3de0676c040d0024416e4f7",
    "comments/wzwso8h2pd/e572aeb8-3464-4303-a64e-8c5de4ee3593.md": "06193004236602b2d282891658435b5edc51da9bae675f649a3a503cb4e4d5d1",
    "comments/wzwso8h2pd/e8e19583-04dd-4d8e-bd52-9f6b99b0e1de.md": "eb0e9efa133d745906785d15d3c9f0aaa76a205a9b9fabc135ac4ef801103923",
    "comments/wzwso8h2pd/eefa70ac-30d4-4e2b-a3b3-8f1142acb389.md": "c3cdc9e9c42e87d0f789b868e890c89d9986c87e80f32c65cc23be7ebcb8bfc8",
    "decisions/312f0ab3-6f63-4ccd-b161-f43d3a874d97.md": "ee69bcbd1892b6328ade9f3a5ad2f32e59aa06bf07e34cd1b4429e0759cd2b96",
    "decisions/6467c7bf-eba5-4b65-a696-9405d6e923c2.md": "736b0bda522ee09e7c0166af00f4dee68779fd9a42d8e712afdde3c3fa238f5e",
    "decisions/e58f2d58-5be1-4265-8363-90d5748ec6f7.md": "f5991cef5da8d8ee27e144dac2e5e6d6c6012f041f01e942311026c529b355d1",
    "scratchpad.md": "77b8d429b065d186d6bbf436d8af228c5e39fbf9419535fe67fba8d5fbda1adb",
    "todos.md": "5c0cad5e53e380372e8e6381b6a605335ff7086deb2b3fa6a50ea467da7d54cb"
  },
  "equal": true
}
```
## B5 Complete shared pressed-action inventory (NR49)

Source search across snapshot src found exactly two `.tab-strip-action` buttons with aria-pressed, both in App.tsx239–240: Notes and Supervisor. Widgets/Commands lack aria-pressed; Browser/Library use `.tab-icon-button` and do not match. Runtime query agreed exactly at open, closed and Supervisor-open states. Mouse parked at(0,0); Notes focus inside Notes, closed focus on the ordinary tab; Supervisor final capture focus inside its header, not on the trigger. Four computed values include background, text color, border color and shadow.

Notes true and false: transparent bg; rgb(186,194,222) text; rgb(43,53,67) border; no shadow. Supervisor false is identical to Notes; true is rgb(35,53,76) bg, rgb(137,180,250) text, rgb(59,71,88) border and inset2px accent edge. Captures: B5-notes-pressed.png, B5-notes-unpressed.png, B5-supervisor-pressed.png.

**Classification:** cross-surface shared App chrome defect, not Notes CSS. Evidence supports the exact approved single rule after parent coordination:

```css
.tab-strip-actions .tab-strip-action[aria-pressed="true"]:not(:disabled){background:var(--select-fill);color:var(--accent);border-color:var(--border-strong);box-shadow:inset 0 var(--select-edge-size) 0 var(--select-edge)}
```

Archived Supervisor-specific equivalent is supervisor.css31; current concurrent source has moved it to248. Same property values preserve its selected appearance; no broader global change is justified. Runtime after-rule parity remains Main's job.

```json
{
  "open": [
    {
      "name": "Notes (open)",
      "controls": "cockpit-notes",
      "class": "tab-strip-action",
      "pressed": "true",
      "background": "rgba(0, 0, 0, 0)",
      "color": "rgb(186, 194, 222)",
      "border": "rgb(43, 53, 67)",
      "shadow": "none",
      "outline": "none",
      "rect": {
        "x": 1184.46875,
        "y": 6,
        "width": 54.203125,
        "height": 28,
        "top": 6,
        "right": 1238.671875,
        "bottom": 34,
        "left": 1184.46875
      }
    },
    {
      "name": "Supervisor",
      "controls": null,
      "class": "tab-strip-action",
      "pressed": "false",
      "background": "rgba(0, 0, 0, 0)",
      "color": "rgb(186, 194, 222)",
      "border": "rgb(43, 53, 67)",
      "shadow": "none",
      "outline": "none",
      "rect": {
        "x": 1350.78125,
        "y": 6,
        "width": 81.21875,
        "height": 28,
        "top": 6,
        "right": 1432,
        "bottom": 34,
        "left": 1350.78125
      }
    }
  ],
  "closed": [
    {
      "name": "Open Notes",
      "controls": "cockpit-notes",
      "class": "tab-strip-action",
      "pressed": "false",
      "background": "rgba(0, 0, 0, 0)",
      "color": "rgb(186, 194, 222)",
      "border": "rgb(43, 53, 67)",
      "shadow": "none",
      "outline": "none",
      "rect": {
        "x": 1184.46875,
        "y": 6,
        "width": 54.203125,
        "height": 28,
        "top": 6,
        "right": 1238.671875,
        "bottom": 34,
        "left": 1184.46875
      }
    },
    {
      "name": "Supervisor",
      "controls": null,
      "class": "tab-strip-action",
      "pressed": "false",
      "background": "rgba(0, 0, 0, 0)",
      "color": "rgb(186, 194, 222)",
      "border": "rgb(43, 53, 67)",
      "shadow": "none",
      "outline": "none",
      "rect": {
        "x": 1350.78125,
        "y": 6,
        "width": 81.21875,
        "height": 28,
        "top": 6,
        "right": 1432,
        "bottom": 34,
        "left": 1350.78125
      }
    }
  ],
  "supervisor": [
    {
      "name": "Open Notes",
      "controls": "cockpit-notes",
      "class": "tab-strip-action",
      "pressed": "false",
      "background": "rgba(0, 0, 0, 0)",
      "color": "rgb(186, 194, 222)",
      "border": "rgb(43, 53, 67)",
      "shadow": "none",
      "outline": "none",
      "rect": {
        "x": 1184.46875,
        "y": 6,
        "width": 54.203125,
        "height": 28,
        "top": 6,
        "right": 1238.671875,
        "bottom": 34,
        "left": 1184.46875
      }
    },
    {
      "name": "Supervisor",
      "controls": null,
      "class": "tab-strip-action",
      "pressed": "true",
      "background": "rgb(35, 53, 76)",
      "color": "rgb(137, 180, 250)",
      "border": "rgb(59, 71, 88)",
      "shadow": "rgb(137, 180, 250) 0px 2px 0px 0px inset",
      "outline": "none",
      "rect": {
        "x": 1350.78125,
        "y": 6,
        "width": 81.21875,
        "height": 28,
        "top": 6,
        "right": 1432,
        "bottom": 34,
        "left": 1350.78125
      }
    }
  ]
}
```
## B6 Comparators, exact measure and focus

- T5 one-line adopted count-free board card: height96.296875px; title31px; empty action row38px. See B6-card-1440.png.
- Short decision footer.top minus **body box**.bottom =12px. The body box stretches656.15625px high; the single rendered paragraph ends at231.09375 and footer starts862.15625, giving a visible content/footer void631.0625px. See B6-short-decision-1440.png. Main was notified that applying `flex:0 1 auto` removes internal body stretch but can preserve the existing12px external gap; relative acceptance should not mistake unchanged external padding for an unchanged void.
- Real source scroller1152px with310px inline padding => measure532px. Content left574, actual source text left580 because CodeMirror cm-line6px inset. See B6-source-1440.png. No widened viewport was needed.

```json
{
  "card": {
    "rect": {
      "x": 265,
      "y": 339.296875,
      "width": 363.328125,
      "height": 96.296875,
      "top": 339.296875,
      "right": 628.328125,
      "bottom": 435.59375,
      "left": 265
    },
    "title": {
      "x": 299,
      "y": 350.296875,
      "width": 287.328125,
      "height": 31,
      "top": 350.296875,
      "right": 586.328125,
      "bottom": 381.296875,
      "left": 299
    },
    "actions": {
      "x": 276,
      "y": 386.59375,
      "width": 341.328125,
      "height": 38,
      "top": 386.59375,
      "right": 617.328125,
      "bottom": 424.59375,
      "left": 276
    }
  },
  "decision_body_box": {
    "body": {
      "x": 554,
      "y": 194,
      "width": 862,
      "height": 656.15625,
      "top": 194,
      "right": 1416,
      "bottom": 850.15625,
      "left": 554
    },
    "footer": {
      "x": 554,
      "y": 862.15625,
      "width": 862,
      "height": 56.84375,
      "top": 862.15625,
      "right": 1416,
      "bottom": 919,
      "left": 554
    },
    "gap": 12
  },
  "decision_content_void": {
    "lastText": {
      "x": 554,
      "y": 208,
      "width": 604.796875,
      "height": 23.09375,
      "top": 208,
      "right": 1158.796875,
      "bottom": 231.09375,
      "left": 554
    },
    "footer": {
      "x": 554,
      "y": 862.15625,
      "width": 862,
      "height": 56.84375,
      "top": 862.15625,
      "right": 1416,
      "bottom": 919,
      "left": 554
    },
    "textGap": 631.0625
  },
  "source": [
    {
      "class": "cm-scroller",
      "rect": {
        "x": 264,
        "y": 170,
        "width": 1152,
        "height": 719,
        "top": 170,
        "right": 1416,
        "bottom": 889,
        "left": 264
      },
      "paddingLeft": "310px",
      "paddingRight": "310px",
      "font": "14px / 19.6px \"IosevkaTerm Nerd Font Mono\", \"IBM Plex Mono\", \"Noto Sans Mono\", monospace"
    },
    {
      "class": "cm-content cm-lineWrapping",
      "rect": {
        "x": 574,
        "y": 170,
        "width": 532,
        "height": 3307,
        "top": 170,
        "right": 1106,
        "bottom": 3477,
        "left": 574
      },
      "paddingLeft": "0px",
      "paddingRight": "0px",
      "font": "15px / 27px \"IBM Plex Sans\", \"Noto Sans\", sans-serif"
    },
    {
      "class": "cm-line",
      "rect": {
        "x": 574,
        "y": 190,
        "width": 532,
        "height": 27,
        "top": 190,
        "right": 1106,
        "bottom": 217,
        "left": 574
      },
      "paddingLeft": "6px",
      "paddingRight": "2px",
      "font": "15px / 27px \"IBM Plex Sans\", \"Noto Sans\", sans-serif"
    },
    {
      "class": "cm-line",
      "rect": {
        "x": 574,
        "y": 217,
        "width": 532,
        "height": 27,
        "top": 217,
        "right": 1106,
        "bottom": 244,
        "left": 574
      },
      "paddingLeft": "6px",
      "paddingRight": "2px",
      "font": "15px / 27px \"IBM Plex Sans\", \"Noto Sans\", sans-serif"
    },
    {
      "class": "cm-line",
      "rect": {
        "x": 574,
        "y": 244,
        "width": 532,
        "height": 27,
        "top": 244,
        "right": 1106,
        "bottom": 271,
        "left": 574
      },
      "paddingLeft": "6px",
      "paddingRight": "2px",
      "font": "15px / 27px \"IBM Plex Sans\", \"Noto Sans\", sans-serif"
    }
  ],
  "source_text": {
    "line": {
      "x": 574,
      "y": 190,
      "width": 532,
      "height": 27,
      "top": 190,
      "right": 1106,
      "bottom": 217,
      "left": 574
    },
    "text": {
      "x": 580,
      "y": 194,
      "width": 377.78125,
      "height": 19,
      "top": 194,
      "right": 957.78125,
      "bottom": 213,
      "left": 580
    }
  }
}
```
### Browser focus sequences

Collected enabled/visible controls, excluded closed-details descendants except their native summaries, hidden/inert ancestors and negative tabindex. Actual trusted Tab/Shift+Tab traversals are recorded, not selector order alone. Programmatic initial focus immediately after a pointer action sometimes has no focus-visible outline; subsequent keyboard traversals show solid outlines. This is baseline semantics, not a newly imposed ring requirement.

Card: handle → checkbox → title → Comments → summary, reverse on Shift+Tab. Closed menu controls are excluded. Picker on real unbound S3: radio → Attach → Cancel when selected; Attach excluded while disabled before selection. No Attach mutation was dispatched. Header at1440/420 has the original Close task details button only; actual Tab exits to the task-title textarea and Shift+Tab returns to Close with solid outline. Agent commands have23 input/copy pairs in the original flat order; actual traversal agrees in both directions. Captures: B6-picker-S3.png (before selection), card/source/short decision above.

```json
{
  "detail_boundary": {
    "exit": {
      "label": "Task title: Adopted board task",
      "tag": "TEXTAREA"
    },
    "back": {
      "label": "Close task details",
      "outline": "solid"
    }
  },
  "sequences": {
    "cardFocus": {
      "selector": ".kanban-card[data-todo-id=\"kjzy16qdk5\"]",
      "DOM": [
        "Move card: One-line no-comment board task",
        "Complete One-line no-comment board task",
        "Task title: One-line no-comment board task",
        "Comments",
        "Card actions: One-line no-comment board task"
      ],
      "forward": [
        {
          "name": "Move card: One-line no-comment board task",
          "outline": "none"
        },
        {
          "name": "Complete One-line no-comment board task",
          "outline": "solid"
        },
        {
          "name": "Task title: One-line no-comment board task",
          "outline": "solid"
        },
        {
          "name": "Comments",
          "outline": "solid"
        },
        {
          "name": "Card actions: One-line no-comment board task",
          "outline": "solid"
        }
      ],
      "Shift_Tab": [
        {
          "name": "Card actions: One-line no-comment board task",
          "outline": "solid"
        },
        {
          "name": "Comments",
          "outline": "solid"
        },
        {
          "name": "Task title: One-line no-comment board task",
          "outline": "solid"
        },
        {
          "name": "Complete One-line no-comment board task",
          "outline": "solid"
        },
        {
          "name": "Move card: One-line no-comment board task",
          "outline": "solid"
        }
      ]
    },
    "picker": {
      "selector": ".notes-binding",
      "DOM": [
        "Last called Polish sample\n2ce09e19-bf76-46d2-af52-129f6a262b8c\nAttached elsewhere",
        "Cancel"
      ],
      "forward": [
        {
          "name": "Last called Polish sample\n2ce09e19-bf76-46d2-af52-129f6a262b8c\nAttached elsewhere",
          "outline": "none"
        },
        {
          "name": "Cancel",
          "outline": "solid"
        }
      ],
      "Shift_Tab": [
        {
          "name": "Cancel",
          "outline": "solid"
        },
        {
          "name": "Last called Polish sample\n2ce09e19-bf76-46d2-af52-129f6a262b8c\nAttached elsewhere",
          "outline": "solid"
        }
      ]
    },
    "pickerSelected": {
      "selector": ".notes-binding",
      "DOM": [
        "Last called Polish sample\n2ce09e19-bf76-46d2-af52-129f6a262b8c\nAttached elsewhere",
        "Attach",
        "Cancel"
      ],
      "forward": [
        {
          "name": "Last called Polish sample\n2ce09e19-bf76-46d2-af52-129f6a262b8c\nAttached elsewhere",
          "outline": "solid"
        },
        {
          "name": "Attach",
          "outline": "solid"
        },
        {
          "name": "Cancel",
          "outline": "solid"
        }
      ],
      "Shift_Tab": [
        {
          "name": "Cancel",
          "outline": "solid"
        },
        {
          "name": "Attach",
          "outline": "solid"
        },
        {
          "name": "Last called Polish sample\n2ce09e19-bf76-46d2-af52-129f6a262b8c\nAttached elsewhere",
          "outline": "solid"
        }
      ]
    },
    "detail1440": {
      "selector": ".notes-task-detail-header",
      "DOM": [
        "Close task details"
      ],
      "forward": [
        {
          "name": "Close task details",
          "outline": "none"
        }
      ],
      "Shift_Tab": [
        {
          "name": "Close task details",
          "outline": "none"
        }
      ]
    },
    "detail420": {
      "selector": ".notes-task-detail-header",
      "DOM": [
        "Close task details"
      ],
      "forward": [
        {
          "name": "Close task details",
          "outline": "solid"
        }
      ],
      "Shift_Tab": [
        {
          "name": "Close task details",
          "outline": "solid"
        }
      ]
    },
    "agentFocus": {
      "selector": ".notes-agent-commands",
      "DOM": [
        "Resolve once, then pin the Notes UUID",
        "Copy Resolve once, then pin the Notes UUID",
        "Scratchpad read",
        "Copy Scratchpad read",
        "Scratchpad append",
        "Copy Scratchpad append",
        "Scratchpad replace (stdin)",
        "Copy Scratchpad replace (stdin)",
        "Todos read",
        "Copy Todos read",
        "Todos add",
        "Copy Todos add",
        "Todos edit",
        "Copy Todos edit",
        "Todos check / reopen",
        "Copy Todos check / reopen",
        "Board read",
        "Copy Board read",
        "Board add",
        "Copy Board add",
        "Board promote",
        "Copy Board promote",
        "Board move",
        "Copy Board move",
        "Board remove (retains todo and comments)",
        "Copy Board remove (retains todo and comments)",
        "Decisions search title and body",
        "Copy Decisions search title and body",
        "Decisions create (Markdown stdin)",
        "Copy Decisions create (Markdown stdin)",
        "Decisions read",
        "Copy Decisions read",
        "Decisions edit",
        "Copy Decisions edit",
        "Decisions replace (old file unchanged)",
        "Copy Decisions replace (old file unchanged)",
        "Comments read",
        "Copy Comments read",
        "Comments post",
        "Copy Comments post",
        "Comments read one",
        "Copy Comments read one",
        "Comments edit",
        "Copy Comments edit",
        "Comments remove",
        "Copy Comments remove"
      ],
      "forward": [
        {
          "name": "Resolve once, then pin the Notes UUID",
          "outline": "solid"
        },
        {
          "name": "Copy Resolve once, then pin the Notes UUID",
          "outline": "solid"
        },
        {
          "name": "Scratchpad read",
          "outline": "solid"
        },
        {
          "name": "Copy Scratchpad read",
          "outline": "solid"
        },
        {
          "name": "Scratchpad append",
          "outline": "solid"
        },
        {
          "name": "Copy Scratchpad append",
          "outline": "solid"
        },
        {
          "name": "Scratchpad replace (stdin)",
          "outline": "solid"
        },
        {
          "name": "Copy Scratchpad replace (stdin)",
          "outline": "solid"
        },
        {
          "name": "Todos read",
          "outline": "solid"
        },
        {
          "name": "Copy Todos read",
          "outline": "solid"
        },
        {
          "name": "Todos add",
          "outline": "solid"
        },
        {
          "name": "Copy Todos add",
          "outline": "solid"
        },
        {
          "name": "Todos edit",
          "outline": "solid"
        },
        {
          "name": "Copy Todos edit",
          "outline": "solid"
        },
        {
          "name": "Todos check / reopen",
          "outline": "solid"
        },
        {
          "name": "Copy Todos check / reopen",
          "outline": "solid"
        },
        {
          "name": "Board read",
          "outline": "solid"
        },
        {
          "name": "Copy Board read",
          "outline": "solid"
        },
        {
          "name": "Board add",
          "outline": "solid"
        },
        {
          "name": "Copy Board add",
          "outline": "solid"
        },
        {
          "name": "Board promote",
          "outline": "solid"
        },
        {
          "name": "Copy Board promote",
          "outline": "solid"
        },
        {
          "name": "Board move",
          "outline": "solid"
        },
        {
          "name": "Copy Board move",
          "outline": "solid"
        },
        {
          "name": "Board remove (retains todo and comments)",
          "outline": "solid"
        },
        {
          "name": "Copy Board remove (retains todo and comments)",
          "outline": "solid"
        },
        {
          "name": "Decisions search title and body",
          "outline": "solid"
        },
        {
          "name": "Copy Decisions search title and body",
          "outline": "solid"
        },
        {
          "name": "Decisions create (Markdown stdin)",
          "outline": "solid"
        },
        {
          "name": "Copy Decisions create (Markdown stdin)",
          "outline": "solid"
        },
        {
          "name": "Decisions read",
          "outline": "solid"
        },
        {
          "name": "Copy Decisions read",
          "outline": "solid"
        },
        {
          "name": "Decisions edit",
          "outline": "solid"
        },
        {
          "name": "Copy Decisions edit",
          "outline": "solid"
        },
        {
          "name": "Decisions replace (old file unchanged)",
          "outline": "solid"
        },
        {
          "name": "Copy Decisions replace (old file unchanged)",
          "outline": "solid"
        },
        {
          "name": "Comments read",
          "outline": "solid"
        },
        {
          "name": "Copy Comments read",
          "outline": "solid"
        },
        {
          "name": "Comments post",
          "outline": "solid"
        },
        {
          "name": "Copy Comments post",
          "outline": "solid"
        },
        {
          "name": "Comments read one",
          "outline": "solid"
        },
        {
          "name": "Copy Comments read one",
          "outline": "solid"
        },
        {
          "name": "Comments edit",
          "outline": "solid"
        },
        {
          "name": "Copy Comments edit",
          "outline": "solid"
        },
        {
          "name": "Comments remove",
          "outline": "solid"
        },
        {
          "name": "Copy Comments remove",
          "outline": "solid"
        }
      ],
      "Shift_Tab": [
        {
          "name": "Copy Comments remove",
          "outline": "solid"
        },
        {
          "name": "Comments remove",
          "outline": "solid"
        },
        {
          "name": "Copy Comments edit",
          "outline": "solid"
        },
        {
          "name": "Comments edit",
          "outline": "solid"
        },
        {
          "name": "Copy Comments read one",
          "outline": "solid"
        },
        {
          "name": "Comments read one",
          "outline": "solid"
        },
        {
          "name": "Copy Comments post",
          "outline": "solid"
        },
        {
          "name": "Comments post",
          "outline": "solid"
        },
        {
          "name": "Copy Comments read",
          "outline": "solid"
        },
        {
          "name": "Comments read",
          "outline": "solid"
        },
        {
          "name": "Copy Decisions replace (old file unchanged)",
          "outline": "solid"
        },
        {
          "name": "Decisions replace (old file unchanged)",
          "outline": "solid"
        },
        {
          "name": "Copy Decisions edit",
          "outline": "solid"
        },
        {
          "name": "Decisions edit",
          "outline": "solid"
        },
        {
          "name": "Copy Decisions read",
          "outline": "solid"
        },
        {
          "name": "Decisions read",
          "outline": "solid"
        },
        {
          "name": "Copy Decisions create (Markdown stdin)",
          "outline": "solid"
        },
        {
          "name": "Decisions create (Markdown stdin)",
          "outline": "solid"
        },
        {
          "name": "Copy Decisions search title and body",
          "outline": "solid"
        },
        {
          "name": "Decisions search title and body",
          "outline": "solid"
        },
        {
          "name": "Copy Board remove (retains todo and comments)",
          "outline": "solid"
        },
        {
          "name": "Board remove (retains todo and comments)",
          "outline": "solid"
        },
        {
          "name": "Copy Board move",
          "outline": "solid"
        },
        {
          "name": "Board move",
          "outline": "solid"
        },
        {
          "name": "Copy Board promote",
          "outline": "solid"
        },
        {
          "name": "Board promote",
          "outline": "solid"
        },
        {
          "name": "Copy Board add",
          "outline": "solid"
        },
        {
          "name": "Board add",
          "outline": "solid"
        },
        {
          "name": "Copy Board read",
          "outline": "solid"
        },
        {
          "name": "Board read",
          "outline": "solid"
        },
        {
          "name": "Copy Todos check / reopen",
          "outline": "solid"
        },
        {
          "name": "Todos check / reopen",
          "outline": "solid"
        },
        {
          "name": "Copy Todos edit",
          "outline": "solid"
        },
        {
          "name": "Todos edit",
          "outline": "solid"
        },
        {
          "name": "Copy Todos add",
          "outline": "solid"
        },
        {
          "name": "Todos add",
          "outline": "solid"
        },
        {
          "name": "Copy Todos read",
          "outline": "solid"
        },
        {
          "name": "Todos read",
          "outline": "solid"
        },
        {
          "name": "Copy Scratchpad replace (stdin)",
          "outline": "solid"
        },
        {
          "name": "Scratchpad replace (stdin)",
          "outline": "solid"
        },
        {
          "name": "Copy Scratchpad append",
          "outline": "solid"
        },
        {
          "name": "Scratchpad append",
          "outline": "solid"
        },
        {
          "name": "Copy Scratchpad read",
          "outline": "solid"
        },
        {
          "name": "Scratchpad read",
          "outline": "solid"
        },
        {
          "name": "Copy Resolve once, then pin the Notes UUID",
          "outline": "solid"
        },
        {
          "name": "Resolve once, then pin the Notes UUID",
          "outline": "solid"
        }
      ]
    }
  }
}
```
### Original Agent command labels and values

```json
[
  {
    "label": "Resolve once, then pin the Notes UUID",
    "value": "COCKPIT_NOTES_ROOT='/tmp/cpol-53ubs82y/data/cockpit/notes/' cockpit-cli notes --current target"
  },
  {
    "label": "Scratchpad read",
    "value": "COCKPIT_NOTES_ROOT='/tmp/cpol-53ubs82y/data/cockpit/notes/' cockpit-cli notes --notes 2ce09e19-bf76-46d2-af52-129f6a262b8c scratchpad read"
  },
  {
    "label": "Scratchpad append",
    "value": "COCKPIT_NOTES_ROOT='/tmp/cpol-53ubs82y/data/cockpit/notes/' cockpit-cli notes --notes 2ce09e19-bf76-46d2-af52-129f6a262b8c scratchpad append --text 'Research notes'"
  },
  {
    "label": "Scratchpad replace (stdin)",
    "value": "COCKPIT_NOTES_ROOT='/tmp/cpol-53ubs82y/data/cockpit/notes/' cockpit-cli notes --notes 2ce09e19-bf76-46d2-af52-129f6a262b8c scratchpad replace --stdin --expected-revision 'sha256:77b8d429b065d186d6bbf436d8af228c5e39fbf9419535fe67fba8d5fbda1adb'"
  },
  {
    "label": "Todos read",
    "value": "COCKPIT_NOTES_ROOT='/tmp/cpol-53ubs82y/data/cockpit/notes/' cockpit-cli notes --notes 2ce09e19-bf76-46d2-af52-129f6a262b8c todo list --open"
  },
  {
    "label": "Todos add",
    "value": "COCKPIT_NOTES_ROOT='/tmp/cpol-53ubs82y/data/cockpit/notes/' cockpit-cli notes --notes 2ce09e19-bf76-46d2-af52-129f6a262b8c todo add --text 'Review API'"
  },
  {
    "label": "Todos edit",
    "value": "COCKPIT_NOTES_ROOT='/tmp/cpol-53ubs82y/data/cockpit/notes/' cockpit-cli notes --notes 2ce09e19-bf76-46d2-af52-129f6a262b8c todo update --ref 'L6@sha256:5c0cad5e53e380372e8e6381b6a605335ff7086deb2b3fa6a50ea467da7d54cb' --text 'Review API contract'"
  },
  {
    "label": "Todos check / reopen",
    "value": "COCKPIT_NOTES_ROOT='/tmp/cpol-53ubs82y/data/cockpit/notes/' cockpit-cli notes --notes 2ce09e19-bf76-46d2-af52-129f6a262b8c todo complete --ref 'L6@sha256:5c0cad5e53e380372e8e6381b6a605335ff7086deb2b3fa6a50ea467da7d54cb'"
  },
  {
    "label": "Board read",
    "value": "COCKPIT_NOTES_ROOT='/tmp/cpol-53ubs82y/data/cockpit/notes/' cockpit-cli notes --notes 2ce09e19-bf76-46d2-af52-129f6a262b8c kanban list"
  },
  {
    "label": "Board add",
    "value": "COCKPIT_NOTES_ROOT='/tmp/cpol-53ubs82y/data/cockpit/notes/' cockpit-cli notes --notes 2ce09e19-bf76-46d2-af52-129f6a262b8c kanban add --text 'Review schema'"
  },
  {
    "label": "Board promote",
    "value": "COCKPIT_NOTES_ROOT='/tmp/cpol-53ubs82y/data/cockpit/notes/' cockpit-cli notes --notes 2ce09e19-bf76-46d2-af52-129f6a262b8c kanban promote --ref 'L6@sha256:5c0cad5e53e380372e8e6381b6a605335ff7086deb2b3fa6a50ea467da7d54cb'"
  },
  {
    "label": "Board move",
    "value": "COCKPIT_NOTES_ROOT='/tmp/cpol-53ubs82y/data/cockpit/notes/' cockpit-cli notes --notes 2ce09e19-bf76-46d2-af52-129f6a262b8c kanban move --ref 'L6@sha256:5c0cad5e53e380372e8e6381b6a605335ff7086deb2b3fa6a50ea467da7d54cb' --to doing"
  },
  {
    "label": "Board remove (retains todo and comments)",
    "value": "COCKPIT_NOTES_ROOT='/tmp/cpol-53ubs82y/data/cockpit/notes/' cockpit-cli notes --notes 2ce09e19-bf76-46d2-af52-129f6a262b8c kanban unboard --ref 'L6@sha256:5c0cad5e53e380372e8e6381b6a605335ff7086deb2b3fa6a50ea467da7d54cb'"
  },
  {
    "label": "Decisions search title and body",
    "value": "COCKPIT_NOTES_ROOT='/tmp/cpol-53ubs82y/data/cockpit/notes/' cockpit-cli notes --notes 2ce09e19-bf76-46d2-af52-129f6a262b8c decision list --status current --query 'schema'"
  },
  {
    "label": "Decisions create (Markdown stdin)",
    "value": "COCKPIT_NOTES_ROOT='/tmp/cpol-53ubs82y/data/cockpit/notes/' cockpit-cli notes --notes 2ce09e19-bf76-46d2-af52-129f6a262b8c decision create --title 'Keep Markdown' --stdin"
  },
  {
    "label": "Decisions read",
    "value": "COCKPIT_NOTES_ROOT='/tmp/cpol-53ubs82y/data/cockpit/notes/' cockpit-cli notes --notes 2ce09e19-bf76-46d2-af52-129f6a262b8c decision get --id <decision-id>"
  },
  {
    "label": "Decisions edit",
    "value": "COCKPIT_NOTES_ROOT='/tmp/cpol-53ubs82y/data/cockpit/notes/' cockpit-cli notes --notes 2ce09e19-bf76-46d2-af52-129f6a262b8c decision update --id <decision-id> --expected-revision '<record-revision>' --title 'Keep ordinary Markdown' --stdin"
  },
  {
    "label": "Decisions replace (old file unchanged)",
    "value": "COCKPIT_NOTES_ROOT='/tmp/cpol-53ubs82y/data/cockpit/notes/' cockpit-cli notes --notes 2ce09e19-bf76-46d2-af52-129f6a262b8c decision replace --id <decision-id> --expected-revision '<record-revision>' --title 'New approach' --stdin"
  },
  {
    "label": "Comments read",
    "value": "COCKPIT_NOTES_ROOT='/tmp/cpol-53ubs82y/data/cockpit/notes/' cockpit-cli notes --notes 2ce09e19-bf76-46d2-af52-129f6a262b8c comment list --todo <todo-id>"
  },
  {
    "label": "Comments post",
    "value": "COCKPIT_NOTES_ROOT='/tmp/cpol-53ubs82y/data/cockpit/notes/' cockpit-cli notes --notes 2ce09e19-bf76-46d2-af52-129f6a262b8c comment add --todo <todo-id> --text 'Blocked on API' --author 'Agent'"
  },
  {
    "label": "Comments read one",
    "value": "COCKPIT_NOTES_ROOT='/tmp/cpol-53ubs82y/data/cockpit/notes/' cockpit-cli notes --notes 2ce09e19-bf76-46d2-af52-129f6a262b8c comment get --todo <todo-id> --comment <comment-id>"
  },
  {
    "label": "Comments edit",
    "value": "COCKPIT_NOTES_ROOT='/tmp/cpol-53ubs82y/data/cockpit/notes/' cockpit-cli notes --notes 2ce09e19-bf76-46d2-af52-129f6a262b8c comment update --todo <todo-id> --comment <comment-id> --expected-revision '<comment-revision>' --stdin"
  },
  {
    "label": "Comments remove",
    "value": "COCKPIT_NOTES_ROOT='/tmp/cpol-53ubs82y/data/cockpit/notes/' cockpit-cli notes --notes 2ce09e19-bf76-46d2-af52-129f6a262b8c comment remove --todo <todo-id> --comment <comment-id> --expected-revision '<comment-revision>'"
  }
]
```
## Reproducible disposable seed

Use unchanged standard helper physically inside an isolated archived snapshot; reuse existing deps, build separate dist once, and use only its returned `/tmp/cpol-*` root. UI explicit Create in helper Space w1; read UUID and Folder from actual Agent access. Pin root/UUID for every CLI call. Additional workspace.create returned w2/w3, with the same fixture repository cwd and focus:false (full DTOs in B1). Their Notes remain unbound.

CLI environment: env -i PATH=<existing PATH> HOME=$FIX XDG_CONFIG_HOME=$FIX/config XDG_STATE_HOME=$FIX/state XDG_CACHE_HOME=$FIX/cache XDG_DATA_HOME=$FIX/data COCKPIT_CONFIG=$FIX/cockpit.toml COCKPIT_NOTES_ROOT=$FIX/data/cockpit/notes. Invoke `$SNAP/target/debug/cockpit notes --notes $UUID` plus each operation below. All revisions/IDs are taken from real JSON replies, never guessed.

1. Scratchpad file is120 lines, exact generator: `''.join(f'Baseline line {i}: readable Markdown source and preview.\n' for i in range(1,121))`; scratchpad read obtains revision; replace --file uses that revision.
2. kanban add Adopted board task → T1. comment add Seed comment author Fixture, then18 more comments with exact body `Overflow comment {i}: synthetic baseline thread content.` for i1..18, author Fixture. Starting count19; enough to overflow real thread.
3. kanban add Doing task → T2; move using returned item revision to doing.
4. todo add Plain todo → T3 (non-board).
5. todo add Completed task → T4; complete using returned item revision.
6. kanban add One-line no-comment board task → T5 (count-free adopted comparator).
7. Append exact UTF-8 bytes below to newline-terminated fixture todos.md (no source/user file):

```markdown
- [ ] Hand-written parent
  - [ ] Nested child one
    - [ ] Nested child two
- [ ] Hand-written unadopted task with a deliberately long title that wraps across two lines in a narrow lane
```

8. D1 decision create title Keep Markdown files, body file `Keep ordinary Markdown files for durable notes.\n`, decided2026-10-01. D2 replace D1 using returned summary.decision_id/revision, title Keep Markdown files, revised, body `Keep ordinary Markdown files with explicit revisions.\n`. D3 decision create title Short decision, file bytes `One line.` (no newline). The installed host supports --file/--stdin for these verbs, not --text.

All actual generated IDs, revisions, hashes and ledger are recorded above. Separate fixture-home Notes root was proved by actual UI Folder, its existing UUID directory, pinned CLI readbacks and HTTP reply identity. No default/user Notes or state/provider files were written.

## Limits and cleanup

Browser-only baseline; no native/WebKitGTK, pointer drag, final behavior tests or final polish acceptance are claimed. The SDK browser worked against actual built source; no mock fallback/install was used. The invalid-date UI recipe discrepancy and the12px box-gap comparator ambiguity are explicit limitations, already notified to Main. Baseline does not replay atlas A01–A19 merely to reconfirm reported failures.

Owned managed tab closed; unchanged helper stop ran only against the exact returned fixture ledger; fixture root removed; read-only host/node_modules symlinks unlinked and exact owned snapshot removed. Shared target/node_modules were not removed. User registry exact bytes still equal. Paths/ports/PIDs/handle in B1 are historical evidence, **not live resources**. No independent worker build/test/lint/formatter was run beyond the single expressly authorized baseline source build.
