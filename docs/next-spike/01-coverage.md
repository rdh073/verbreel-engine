# Next Spike Coverage Table

- Raw command headers audited: 128
- Unique command verb ids audited: 127
- Conformance baseline accepted for this spike: `cargo run -p verbreel-conformance` => `PASS — 121 verbs, 121 fixtures`.
- Rows with `LOC=0 FILES=0 CALL_SITES=0` are already conformance-green; their class is `MECHANICAL` because no product decision or upstream dependency is attached.

| § | verb_id | crate(s) | spec status | engine status | gap | LOC | FILES | CALL_SITES | class |
|---|---------|----------|-------------|----------------|-----|-----|-------|------------|-------|
| 3.1 | `asset.import` | verbreel-state, verbreel-storage, verbreel-codec-native | v1.1 command spec asset.md:3 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 3.2 | `asset.list` | verbreel-state, verbreel-storage, verbreel-codec-native | v1.1 command spec asset.md:34 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 3.3 | `asset.probe` | verbreel-state, verbreel-storage, verbreel-codec-native | v1.1 command spec asset.md:43 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 3.4 | `asset.remove` | verbreel-state, verbreel-storage, verbreel-codec-native | v1.1 command spec asset.md:53 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 3.5 | `asset.relink` | verbreel-state, verbreel-storage, verbreel-codec-native | v1.1 command spec asset.md:71 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 3.6 | `asset.gc` | verbreel-state, verbreel-storage, verbreel-codec-native | v1.1 command spec asset.md:88 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 3.7 | `asset.verify` | verbreel-state, verbreel-storage, verbreel-codec-native | v1.1 command spec asset.md:108 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 19.1 | `audio.detect_beats` | verbreel-state, verbreel-codec-native, verbreel-ai | v1.1 command spec audio-analysis.md:21 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 19.2 | `audio.analyze` | verbreel-state, verbreel-codec-native, verbreel-ai | v1.1 command spec audio-analysis.md:178 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 19.3 | `audio.detect_silence` | verbreel-state, verbreel-codec-native, verbreel-ai | v1.1 command spec audio-analysis.md:258 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 19.7 | `list_capabilities` | verbreel-state | v1.1 additive fields on §1.5 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 9.1 | `audio.extract` | verbreel-state, verbreel-storage, verbreel-codec-native | v1.1 command spec audio.md:3 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 9.2 | `audio.volume` | verbreel-state, verbreel-codec-native | v1.1 command spec audio.md:19 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 9.3 | `audio.fade` | verbreel-state, verbreel-codec-native | v1.1 command spec audio.md:29 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 9.4 | `audio.denoise` | verbreel-state, verbreel-codec-native | v1.1 command spec audio.md:45 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 10.1 | `caption.auto_generate` | verbreel-state, verbreel-ai | v1.1 command spec caption.md:3 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 10.2 | `caption.edit` | verbreel-state | v1.1 command spec caption.md:37 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 10.3 | `caption.translate` | verbreel-state, verbreel-ai | v1.1 command spec caption.md:47 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 10.4 | `caption.burn_in` | verbreel-state, verbreel-render, verbreel-codec-native | v1.1 command spec caption.md:64 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 10.5 | `caption.burn_off` | verbreel-state, verbreel-render, verbreel-codec-native | v1.1 command spec caption.md:94 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 10.6 | `caption.export` | verbreel-state, verbreel-render, verbreel-codec-native | v1.1 command spec caption.md:115 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 5.1 | `clip.add` | verbreel-state | v1.1 command spec clip.md:3 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 5.2 | `clip.trim` | verbreel-state | v1.1 command spec clip.md:73 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 5.3 | `clip.split` | verbreel-state | v1.1 command spec clip.md:92 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 5.4 | `clip.move` | verbreel-state | v1.1 command spec clip.md:135 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 5.5 | `clip.delete` | verbreel-state | v1.1 command spec clip.md:154 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 5.6 | `clip.duplicate` | verbreel-state | v1.1 command spec clip.md:190 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 5.7 | `clip.set_speed` | verbreel-state | v1.1 command spec clip.md:212 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 5.8 | `clip.reverse` | verbreel-state | v1.1 command spec clip.md:264 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 5.9 | `clip.set_transform` | verbreel-state | v1.1 command spec clip.md:276 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 5.10 | `clip.set_opacity` | verbreel-state | v1.1 command spec clip.md:286 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 5.11 | `clip.set_volume` | verbreel-state | v1.1 command spec clip.md:294 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 5.12 | `clip.set_fade` | verbreel-state | v1.1 command spec clip.md:304 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 5.13 | `clip.lock` | verbreel-state | v1.1 command spec clip.md:328 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 5.14 | `clip.list` | verbreel-state | v1.1 command spec clip.md:339 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 5.16 | `clip.unlink` | verbreel-state | v1.1 command spec clip.md:419 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 5.17 | `clip.rename` | verbreel-state | v1.1 command spec clip.md:429 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 5.18 | `clip.set_blend_mode` | verbreel-state | v1.1 command spec clip.md:441 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 5.19 | `clip.set_mask` | verbreel-state | v1.1 command spec clip.md:458 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 5.20 | `clip.set_speed_curve` | verbreel-state | v1.1 command spec clip.md:501 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 20.1 | `compound.create` | verbreel-state | v1.1 command spec compound.md:11 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 20.2 | `compound.expand` | verbreel-state | v1.1 command spec compound.md:71 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 20.3 | `compound.flatten` | verbreel-state | v1.1 command spec compound.md:91 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 20.4 | `compound.edit_in_place` | verbreel-state | v1.1 command spec compound.md:106 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 6.1 | `effect.add` | verbreel-state | v1.1 command spec effect.md:3 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 6.2 | `effect.remove` | verbreel-state | v1.1 command spec effect.md:24 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 6.3 | `effect.set_param` | verbreel-state | v1.1 command spec effect.md:43 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 6.4 | `effect.toggle` | verbreel-state | v1.1 command spec effect.md:51 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 6.5 | `effect.list_available` | verbreel-state | v1.1 command spec effect.md:61 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 6.6 | `effect.reorder` | verbreel-state | v1.1 command spec effect.md:128 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 8.1 | `keyframe.add` | verbreel-state | v1.1 command spec keyframe.md:3 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 8.2 | `keyframe.remove` | verbreel-state | v1.1 command spec keyframe.md:38 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 8.3 | `keyframe.set` | verbreel-state | v1.1 command spec keyframe.md:46 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 8.4 | `keyframe.list` | verbreel-state | v1.1 command spec keyframe.md:54 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 13.1 | `marker.add` | verbreel-state | v1.1 command spec marker.md:5 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 13.2 | `marker.set` | verbreel-state | v1.1 command spec marker.md:13 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 13.3 | `marker.remove` | verbreel-state | v1.1 command spec marker.md:32 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 13.4 | `marker.list` | verbreel-state | v1.1 command spec marker.md:40 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 1.1 | `help` | verbreel-state | v1.1 command spec meta.md:3 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 1.2 | `schema` | verbreel-state, verbreel-args | v1.1 command spec meta.md:19 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 1.3 | `describe` | verbreel-state | v1.1 command spec meta.md:29 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 1.4 | `validate_command` | verbreel-state, verbreel-args | v1.1 command spec meta.md:53 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 1.5 | `list_capabilities` | verbreel-state | v1.1 command spec meta.md:63 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 15.1 | `preview.session.create` | verbreel-state, verbreel-render, verbreel-codec-web, verbreel-wasm | v1.1 command spec preview-session.md:11 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 15.2 | `preview.session.seek` | verbreel-state, verbreel-render, verbreel-codec-web, verbreel-wasm | v1.1 command spec preview-session.md:34 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 15.3 | `preview.session.play` | verbreel-state, verbreel-render, verbreel-codec-web, verbreel-wasm | v1.1 command spec preview-session.md:51 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 15.4 | `preview.session.pause` | verbreel-state, verbreel-render, verbreel-codec-web, verbreel-wasm | v1.1 command spec preview-session.md:82 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 15.5 | `preview.session.close` | verbreel-state, verbreel-render, verbreel-codec-web, verbreel-wasm | v1.1 command spec preview-session.md:95 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 15.6 | `preview.session.frame_at` | verbreel-state, verbreel-render, verbreel-codec-web, verbreel-wasm | v1.1 command spec preview-session.md:107 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 14.1 | `preview.frame` | verbreel-state, verbreel-render, verbreel-codec-web, verbreel-wasm | v1.1 command spec preview.md:5 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 14.2 | `preview.waveform` | verbreel-state, verbreel-render, verbreel-codec-web, verbreel-wasm | v1.1 command spec preview.md:41 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 14.3 | `preview.thumbnail` | verbreel-state, verbreel-render, verbreel-codec-web, verbreel-wasm | v1.1 command spec preview.md:64 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 2.1 | `project.create` | verbreel-state, verbreel-storage, verbreel-events | v1.1 command spec project.md:3 | module + test present; not in default_registry/conformance; prior issue #231 closed | Module + test exist, but default_registry()/conformance exclude native lifecycle creation; fold into Tier 1-B lifecycle migration. | 70 | 2 | 1 | MECHANICAL |
| 2.2 | `project.open` | verbreel-state, verbreel-storage, verbreel-events | v1.1 command spec project.md:37 | module + test present; not in default_registry/conformance; prior issue #233 closed | Module + test exist, but default_registry()/conformance exclude native lifecycle open; fold into Tier 1-B lifecycle migration. | 90 | 3 | 2 | MECHANICAL |
| 2.3 | `project.save` | verbreel-state, verbreel-storage, verbreel-events | v1.1 command spec project.md:73 | module + test present; not in default_registry/conformance; prior issue #234 closed | Module + test exist, but default_registry()/conformance exclude native lifecycle save; fold into Tier 1-B lifecycle migration. | 80 | 3 | 2 | MECHANICAL |
| 2.4 | `project.info` | verbreel-state, verbreel-storage, verbreel-events | v1.1 command spec project.md:92 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 2.5 | `project.close` | verbreel-state, verbreel-storage, verbreel-events | v1.1 command spec project.md:116 | module + test present; not in default_registry/conformance; prior issue #236 closed | Module + test exist, but default_registry()/conformance exclude native lifecycle close; fold into Tier 1-B lifecycle migration. | 70 | 2 | 1 | MECHANICAL |
| 2.6 | `project.list` | verbreel-state, verbreel-storage, verbreel-events | v1.1 command spec project.md:139 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 2.7 | `project.duplicate` | verbreel-state, verbreel-storage, verbreel-events | v1.1 command spec project.md:171 | module + test present; not in default_registry/conformance; prior issue #239 closed | Module + test exist, but default_registry()/conformance exclude native lifecycle duplicate; fold into Tier 1-B lifecycle migration. | 110 | 4 | 3 | MECHANICAL |
| 2.8 | `project.forget` | verbreel-state, verbreel-storage, verbreel-events | v1.1 command spec project.md:194 | module + test present; not in default_registry/conformance; prior issue #240 closed | Module + test exist, but default_registry()/conformance exclude native lifecycle forget; fold into Tier 1-B lifecycle migration. | 60 | 2 | 1 | MECHANICAL |
| 2.9 | `project.rename` | verbreel-state, verbreel-storage, verbreel-events | v1.1 command spec project.md:204 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 2.10 | `project.set_canvas` | verbreel-state, verbreel-storage, verbreel-events | v1.1 command spec project.md:214 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 2.11 | `project.set_fps` | verbreel-state, verbreel-storage, verbreel-events | v1.1 command spec project.md:227 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 2.12 | `project.set_metadata` | verbreel-state, verbreel-storage, verbreel-events | v1.1 command spec project.md:239 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 21.1 | `render.queue.add` | verbreel-state, verbreel-render | v1.1 command spec render-queue.md:23 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 21.2 | `render.queue.list` | verbreel-state, verbreel-render | v1.1 command spec render-queue.md:119 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 21.3 | `render.queue.status` | verbreel-state, verbreel-render | v1.1 command spec render-queue.md:162 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 21.4 | `render.queue.cancel` | verbreel-state, verbreel-render | v1.1 command spec render-queue.md:183 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 21.5 | `render.queue.clear` | verbreel-state, verbreel-render | v1.1 command spec render-queue.md:209 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 11.1 | `render.start` | verbreel-state, verbreel-render, verbreel-codec-native | v1.1 command spec render.md:3 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 11.2 | `render.status` | verbreel-state, verbreel-render, verbreel-codec-native | v1.1 command spec render.md:64 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 11.3 | `render.cancel` | verbreel-state, verbreel-render, verbreel-codec-native | v1.1 command spec render.md:85 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 11.4 | `render.list_presets` | verbreel-state, verbreel-render, verbreel-codec-native | v1.1 command spec render.md:97 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 17.1 | `stock.list_providers` | verbreel-state | v1.1 command spec stock.md:18 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 17.2 | `stock.search` | verbreel-state | v1.1 command spec stock.md:55 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 17.3 | `stock.import` | verbreel-state | v1.1 command spec stock.md:132 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 17.4 | `stock.describe` | verbreel-state | v1.1 command spec stock.md:230 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 16.1 | `template.list` | verbreel-state | v1.1 command spec template.md:144 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 16.2 | `template.describe` | verbreel-state | v1.1 command spec template.md:182 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 16.3 | `template.apply` | verbreel-state | v1.1 command spec template.md:232 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 16.4 | `template.from_project` | verbreel-state | v1.1 command spec template.md:320 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 16.5 | `template.install` | verbreel-state | v1.1 command spec template.md:385 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 16.6 | `template.uninstall` | verbreel-state | v1.1 command spec template.md:411 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 7.1 | `text.add` | verbreel-state | v1.1 command spec text.md:5 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 7.2 | `text.edit` | verbreel-state | v1.1 command spec text.md:36 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 7.3 | `text.style` | verbreel-state | v1.1 command spec text.md:46 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 7.4 | `text.animate` | verbreel-state | v1.1 command spec text.md:60 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 7.5 | `font.list` | verbreel-state | v1.1 command spec text.md:77 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 12.1 | `timeline.snapshot` | verbreel-state | v1.1 command spec timeline.md:3 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 12.2 | `timeline.diff` | verbreel-state | v1.1 command spec timeline.md:14 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 12.3 | `timeline.undo` | verbreel-state | v1.1 command spec timeline.md:30 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 12.4 | `timeline.redo` | verbreel-state | v1.1 command spec timeline.md:47 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 12.6 | `timeline.history` | verbreel-state | v1.1 command spec timeline.md:91 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 4.1 | `track.add` | verbreel-state | v1.1 command spec track.md:3 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 4.2 | `track.remove` | verbreel-state | v1.1 command spec track.md:21 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 4.3 | `track.reorder` | verbreel-state | v1.1 command spec track.md:40 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 4.4 | `track.mute` | verbreel-state | v1.1 command spec track.md:52 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 4.5 | `track.solo` | verbreel-state | v1.1 command spec track.md:64 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 4.6 | `track.lock` | verbreel-state | v1.1 command spec track.md:89 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 4.7 | `track.rename` | verbreel-state | v1.1 command spec track.md:100 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 4.8 | `track.set_volume` | verbreel-state | v1.1 command spec track.md:108 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 4.9 | `track.set_pan` | verbreel-state | v1.1 command spec track.md:118 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 4.10 | `track.hide` | verbreel-state | v1.1 command spec track.md:128 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 18.1 | `tracker.create` | verbreel-state, verbreel-ai | v1.1 command spec tracker.md:49 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 18.2 | `tracker.run` | verbreel-state, verbreel-ai | v1.1 command spec tracker.md:97 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 18.3 | `tracker.apply` | verbreel-state, verbreel-ai | v1.1 command spec tracker.md:214 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 18.4 | `tracker.list` | verbreel-state, verbreel-ai | v1.1 command spec tracker.md:300 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |
| 18.5 | `tracker.remove` | verbreel-state, verbreel-ai | v1.1 command spec tracker.md:348 | implemented; default_registry + fixture; conformance PASS | none; conformance-green | 0 | 0 | 0 | MECHANICAL |

## Coverage Gap Rows

- `project.create` §2.1: Module + test exist, but default_registry()/conformance exclude native lifecycle creation; fold into Tier 1-B lifecycle migration. Sizing: LOC=70, FILES=2, CALL_SITES=1, class=MECHANICAL. Issue: #231 closed.
- `project.open` §2.2: Module + test exist, but default_registry()/conformance exclude native lifecycle open; fold into Tier 1-B lifecycle migration. Sizing: LOC=90, FILES=3, CALL_SITES=2, class=MECHANICAL. Issue: #233 closed.
- `project.save` §2.3: Module + test exist, but default_registry()/conformance exclude native lifecycle save; fold into Tier 1-B lifecycle migration. Sizing: LOC=80, FILES=3, CALL_SITES=2, class=MECHANICAL. Issue: #234 closed.
- `project.close` §2.5: Module + test exist, but default_registry()/conformance exclude native lifecycle close; fold into Tier 1-B lifecycle migration. Sizing: LOC=70, FILES=2, CALL_SITES=1, class=MECHANICAL. Issue: #236 closed.
- `project.duplicate` §2.7: Module + test exist, but default_registry()/conformance exclude native lifecycle duplicate; fold into Tier 1-B lifecycle migration. Sizing: LOC=110, FILES=4, CALL_SITES=3, class=MECHANICAL. Issue: #239 closed.
- `project.forget` §2.8: Module + test exist, but default_registry()/conformance exclude native lifecycle forget; fold into Tier 1-B lifecycle migration. Sizing: LOC=60, FILES=2, CALL_SITES=1, class=MECHANICAL. Issue: #240 closed.

## Audit Notes

- The six `project.*` lifecycle verbs have state files and tests, but only `project.info`, `project.list`, `project.rename`, `project.set_canvas`, `project.set_fps`, and `project.set_metadata` are in `default_registry()`. The first invariant break for conformance coverage is registry membership, not the per-verb files.
- `audio-analysis.md` §19.7 is an additive `list_capabilities` shape section, not a second verb implementation; the current implementation advertises `audio_analysis_algorithms` and `audio_analysis_features`.
- `compound.edit_in_place_end` appears only as prose in the §20.4 heading. The registered verb id is `compound.edit_in_place`; no second command id exists in the v1.1 table.
