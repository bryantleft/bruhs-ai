"""Render the ongoing action log without promoting partial/component evidence.

Usage: python3 scripts/performance-action-log.py --ledger PATH --markdown PATH --json PATH
The source catalogue is discovery evidence. It is not an exhaustive semantic inventory.
"""
from collections import Counter
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
from performance_evidence import PUBLIC_SUMMARY, assert_publishable, load_ledger, make_summary

ROOT=Path(__file__).resolve().parent.parent
parser=argparse.ArgumentParser()
parser.add_argument('--ledger',default=PUBLIC_SUMMARY,help='Full local ledger or compact public index')
parser.add_argument('--summary',help='Optional compact public index output; full input must be archived under target/')
parser.add_argument('--inventory',default='target/performance-action-inventory.json')
parser.add_argument('--comparisons',default='target/performance-t3-gaps1-matrix.json')
parser.add_argument('--discovery-receipt',help='Pinned receipt for a newer source discovery snapshot')
parser.add_argument('--markdown',default='docs/performance-action-log.md')
parser.add_argument('--json',dest='json_path',default='docs/performance-action-log.json')
args=parser.parse_args()
link=lambda p:os.path.relpath(ROOT/p,(ROOT/args.markdown).parent)
read=lambda p:json.loads((ROOT/p).read_text())
sha=lambda p:hashlib.sha256((ROOT/p).read_bytes()).hexdigest()
ledger_path,ledger=load_ledger(ROOT,args.ledger)
inventory,comparisons=read(args.inventory),read(args.comparisons)
# Presentation-only spacing corrections; preserve the pinned comparison input.
for target in comparisons['targets']:
    for before,after in [('measures231.85ms','measures 231.85 ms'),('versus239.85ms','versus 239.85 ms'),('T3server','T3 server')]:
        target['finding']=target['finding'].replace(before,after)
identities={(i['kind'],i['id']):i for i in inventory['items'] if i['kind']!='binding_reference'}
assert len(identities)==len([i for i in inventory['items'] if i['kind']!='binding_reference'])
entries={k:[] for k in identities}
observations=[]
request_counts=[]
body_observations=[]
body_files={}
body_observation_ids=set()
raw_cache={}
environments={}
for e in ledger['entries']:
    key=(e['kind'],e['id']);assert key in entries
    entries[key].append(e)
    body_path=e.get('network_body_observation_file')
    if body_path:
        if body_path not in body_files:
            body=read(body_path)
            assert body['version']==1 and body['status']=='passed'
            body_files[body_path]=(body,sha(body_path))
        body=body_files[body_path][0]
        matching=[o for o in body['observations'] if (o['kind'],o['id'])==key]
        assert matching, 'No body observations match '+e['evidence_id']
        for o in matching:
            assert o['observation_id'] not in body_observation_ids, 'Duplicate body observation'
            assert o['unit']=='bytes' and o['samples']==1
            assert o['wire_bytes'] is None and o['latency_ms'] is None
            for field in ['response_body_bytes_written','browser_response_body_bytes_received','fixture_body_bytes']:
                assert type(o[field]) is int and o[field]>=0, field
            assert o['response_body_bytes_written']==o['browser_response_body_bytes_received']<=o['fixture_body_bytes']
            assert o['whole_action_acceptance'] is False and o['t3_comparable'] is False
            if o['raw_artifact'] not in raw_cache:
                raw_cache[o['raw_artifact']]=(read(o['raw_artifact']),sha(o['raw_artifact']))
            raw,digest=raw_cache[o['raw_artifact']]
            assert digest==o['raw_sha256']
            row=raw
            for part in o['raw_pointer']:row=row[part]
            assert row==o['response_body_bytes_written'], 'Raw response-body count changed'
            body_observations.append(o)
            body_observation_ids.add(o['observation_id'])
    for path in e.get('timing_observation_files',[]):
        normalized=read(path)
        assert normalized['version']==1 and normalized['evidence_id']==e['evidence_id']
        environments[e['evidence_id']]=normalized.get('environment_observed_after_run',{})
        for o in normalized['observations']:
            assert (o['kind'],o['id'])==key and o['unit']=='ms' and o['samples']>0
            if o['raw_artifact'] not in raw_cache:
                raw_cache[o['raw_artifact']]=(read(o['raw_artifact']),sha(o['raw_artifact']))
                for source,digest in raw_cache[o['raw_artifact']][0].get('raw_inputs_sha256',{}).items():
                    assert sha(source)==digest,'Original timing input changed: '+source
            raw,digest=raw_cache[o['raw_artifact']]
            assert digest==o['raw_sha256']
            row=raw
            for part in o['raw_pointer']:row=row[part]
            assert row['summary_ms']==o['quantiles'] and len(row['samples_ms'])==o['samples']
            assert max(row['samples_ms'])==o['max']
            assert sha(o['run_receipt'])==o['run_receipt_sha256']
            assert all(math.isfinite(value) and value>=0 for value in o['quantiles'].values())
            assert o['whole_action_acceptance'] is False
            observations.append({**o,'evidence_id':e['evidence_id']})
    for path in e.get('request_count_observation_files',[]):
        counted=read(path)
        assert counted['version']==1 and counted['evidence_id']==e['evidence_id']
        environments[e['evidence_id']]=counted['environment']
        for o in counted['observations']:
            assert (o['kind'],o['id'])==key and o['samples']==1 and o['latency_ms'] is None and o['wire_bytes'] is None
            raw=read(o['raw_artifact']);assert sha(o['raw_artifact'])==o['raw_sha256']
            if o.get('raw_profile')=='real_runtime_sse':
                assert raw['passed'] and raw['terminal']=={'code':0,'signal':None} and not raw['browser_errors']
                assert o['raw_pointer'][0]=='keepalives'
                row=raw
                for part in o['raw_pointer']:row=row[part]
                assert row['variant']==o['variant'] and row['profile']==o['network_profile']
                assert row['snapshot_requests']==o['snapshot_requests'] and isinstance(o['snapshot_requests'],int) and o['snapshot_requests']>=0
                assert len(row['read_chunks'])==o['observed_read_chunks']
                assert all(chunk['text']==':\n\n' and chunk['bytes']==3 and not chunk['done'] for chunk in row['read_chunks'])
                assert o['response_body_bytes'] is None and o['whole_action_acceptance'] is False and o['t3_comparable'] is False
                request_counts.append({**o,'evidence_id':e['evidence_id']})
                continue
            assert raw['pass'] and raw['browserClosed'] and raw['serverClosed'] and not raw['errors']
            assert o['raw_pointer'][0]=='cohorts' and o['raw_pointer'][2]=='cases'
            cohort=raw['cohorts'][o['raw_pointer'][1]]
            assert cohort['variant']==o['variant'] and cohort['width']==o['viewport_width'] and cohort['local']==o['local_listener']
            row=raw
            for part in o['raw_pointer']:row=row[part]
            assert row['name']==o['workload']
            for field in ['source_chunks','snapshot_requests','response_body_bytes']:
                assert row[field]==o[field] and isinstance(o[field],int) and o[field]>=0
            assert o['whole_action_acceptance'] is False and o['t3_comparable'] is False
            request_counts.append({**o,'evidence_id':e['evidence_id']})
for body,_ in body_files.values():
    assert all(o['observation_id'] in body_observation_ids for o in body['observations']), 'Unmapped body observation'
body_observations=[o for body,_ in body_files.values() for o in body['observations']]
actions=[]
for key,item in identities.items():
    linked=entries[key]
    measured=[o for o in observations if (o['kind'],o['id'])==key]
    counted=[o for o in request_counts if (o['kind'],o['id'])==key]
    functional_only=bool(linked) and all(e.get('measurement_kind') in {'functional','functional_only','functional_and_bytes'} for e in linked)
    actions.append({'kind':key[0],'id':key[1],'source':item['source'],'line':item['line'],
        'semantic_inventory_review':'pending','source_status':item['measurement_status'],
        'evidence_ids':[e['evidence_id'] for e in linked],
        'normalized_observation_ids':[o['observation_id'] for o in measured],
        'request_count_observation_ids':[o['observation_id'] for o in counted],
        'timing_status':'bounded_variant_observations' if measured else ('request_counts_recorded_latency_pending' if counted else ('functional_checks_recorded_latency_pending' if functional_only else ('historical_timing_extraction_pending' if linked else 'measurement_pending'))),
        'whole_action_acceptance':False})
result={'version':1,'completion_status':'in_progress','semantic_inventory_complete':False,
    'purpose':'Living action timing log requested by the user. Preserve workload/boundary/source context; unknown timings are explicit, never zero or inferred from component results.',
    'counts':{'source_entrypoints':len(actions),'binding_search_references':inventory['counts'].get('binding_reference',0),
              'partial_evidence_records':len(ledger['entries']),'normalized_observations':len(observations),'request_count_observations':len(request_counts),
              'whole_action_acceptances':0,'verified_faster_t3_targets':comparisons['comparison_wins_verified']},
    'inputs_sha256':{'ledger':hashlib.sha256(ledger_path.read_bytes()).hexdigest(),'inventory':sha(args.inventory),'comparisons':sha(args.comparisons)},
    'actions':actions,'observations':observations,'request_count_observations':request_counts,'measurement_environments':environments,'t3_comparisons':comparisons['targets'],
    'remaining':['Review semantic user actions beyond source-discovered identities, including keyboard/pointer/context-menu, lifecycle/error/cancellation/retry variants and all supported clients.',
                 'Normalize historical timing receipts into typed observations with exact raw pointers.',
                 'Measure unmatched actions and T3 workloads with equivalent data, functionality, clocks and network conditions.',
                 'Verify final build/source context and every required action before declaring completion.']}
semantic_catalogue=read('docs/performance-action-catalogue.json')
result['source_profiles']=semantic_catalogue['source_profiles']
result['counts'].update(reviewed_semantic_intents=len(semantic_catalogue['actions']),response_body_byte_observations=len(body_observations))
result['response_body_byte_observations']=body_observations
result['inputs_sha256']['network_body_observations']={path:digest for path,(_,digest) in body_files.items()}
result['response_body_observation_scope']='Functional response-body observations only. No request/header/TCP/TLS wire total or normalized latency; complete-action and T3 acceptance remain open.'
result['inputs_sha256']['semantic_catalogue']=sha('docs/performance-action-catalogue.json')
result['landings']=[{k:v for k,v in landing.items() if k in ['id','pull_request','merge_commit','tested_and_merged_tree','report','root_integration']} for landing in ledger['landings']]
result['discovery_supplements']=ledger.get('discovery_supplements',[])
for supplement in result['discovery_supplements']:
    assert sha(supplement['artifact'])==supplement['sha256'], 'Discovery supplement changed'
discovery_note='A later current-root scan stopped at an unsupported router composition. A [source-pinned supplement]('+link('target/performance-envelope3-network-discovery-supplement.json')+') records nine delivery routes and seven routes on the optional paired-task listener in unpublished work, outside the frozen 276 identities. These additions remain unmeasured; this is not a complete source refresh.'
if args.discovery_receipt:
    discovery=read(args.discovery_receipt)
    assert sha(args.inventory)==discovery['inventory_sha256']
    assert discovery['entrypoints']==len(actions) and discovery['discovery_fields_reproduced']
    result['source_discovery_snapshot']={'receipt':args.discovery_receipt,'receipt_sha256':sha(args.discovery_receipt),**discovery}
    result['remaining'].insert(0,'Review newly discovered network, palette and view entrypoints semantically and measure their required variants; source discovery does not establish supported-action or network completeness.')
    delivery=sum(i['kind']=='http' and i['source']=='bruhs-server/src/routes/delivery.rs' for i in inventory['items'])
    hosts=sum(i['kind']=='http' and i['source']=='bruhs-server/src/routes/remote_hosts.rs' for i in inventory['items'])
    paired=sum(i['kind']=='http' and i.get('listener')=='paired_task' for i in inventory['items'])
    discovery_note=f"The [newer source-pinned discovery capture]({link(args.discovery_receipt)}) includes {len(actions)} entrypoints from {discovery['source_files']} files: {delivery} delivery routes, {hosts} local remote-host routes and {paired} routes on the optional paired-task listener. Listener identities remain distinct. The isolated replay reproduces discovery fields; historical availability metadata stays separate. Exact source bytes are retained, but this text scan is not an atomic build snapshot, complete semantic action inventory or network acceptance."
    if discovery.get('in_progress_owner_scope'):
        discovery_note+=' '+discovery['in_progress_owner_scope']
elif result['discovery_supplements']:
    result['remaining'].insert(0, 'Refresh current-root source discovery, including delivery router composition and the optional paired-task listener; the frozen 276-entrypoint snapshot omits these additions.')
escape=lambda s:str(s).replace('|','\\|').replace('\n',' ')
lines=['# Action timing log (incomplete)','',
    'The performance goal is active. This log lists discovered entrypoints and bounded measurements; it does not establish that every semantic action is catalogued or fast. The final version must include each supported action and its verified timings.','',
    f"The catalogue has **{len(actions)} source entrypoints** plus {result['counts']['binding_search_references']} binding search references awaiting semantic review. There are {len(ledger['entries'])} partial evidence records and {len(observations)} normalized timing observations from bounded studies, plus {len(request_counts)} separate request-count cases without latency measurements. Whole-action acceptances and matched faster-than-T3 results are both zero. Older records remain linked while their numerical timings are normalized.",'',
    'Normalized observations are typed timing groups. Different clocks can reuse the same underlying intervals, so this total is not a count of independent trials.','',
    f"The 45 reviewed command intents retain their earlier source profiles: published main `{semantic_catalogue['source_profiles']['published_main']['base_revision'][:8]}` and local working-tree base `{semantic_catalogue['source_profiles']['working_tree']['base_revision'][:8]}`. New source discovery does not revalidate that semantic review. Timing rows keep their own captured build hashes. Concurrent edits after each capture are outside its snapshot. Local main contains concurrent unpublished work and has not integrated the latest published performance merge.",'',
    discovery_note,'',
    '## T3 comparison targets','',
    'The screenshot mixes server reads, formatter CPU, bootstrap bytes and impaired-link completion. Each target keeps its original boundary. Source inspection identifies structural differences; it does not establish a timing win.','',
    '| Action | T3 reported | Bruhs finding | Required measurement |','| --- | ---: | --- | --- |']
for t in comparisons['targets']:
    lines.append(f"| {escape(t['action'])} | [{escape(t.get('t3_boundary_short',t['t3_reported']))}]({t['source_pr']}) | {escape(t['finding'])} | {escape(t['needed'])} |")
lines += ['', '## Current bounded network timings','',
    'GET /api/meetings/stream, synthetic 16-event batches in 13-byte producer fragments. Each row is a separate process/group with 256 retained samples; p99 is its third-largest observation. These include two loopback TCP hops and exclude physical UI, real capture and remote DNS/TLS. They remain partial network variants.','',
    '`cursor` means compaction/borrowed frames only; `scan` means forward scanning only; `combined` includes both. Whole-batch input sends all event frames as one producer chunk; fragmented input uses at most 13 bytes per producer chunk. Both complete-delivery cases time receipt/framing of the final expected event; actual TCP chunk boundaries are not asserted.','',
    '| Variant | Group | p50 ms | p95 ms | p99 ms | Raw evidence |','| --- | --- | ---: | ---: | ---: | --- |']
for o in observations:
    if o['measurement_level']=='network_variant' and o['workload']=='two-hop/16/fragmented':
        q=o['quantiles'];lines.append(f"| {o['variant']} | {o['group']} | {q['p50']:.6f} | {q['p95']:.6f} | {q['p99']:.6f} | [{o['evidence_id']}]({link(o['raw_artifact'])}) |")
for title,evidence_id,scope in [
    ('Thread opening trial','web-thread-open-row-index1-2026-11-13','Row-index candidate not adopted. Actual sidebar input through normal IndexedDB history, final-row layout and two frames. Six retained samples per cell; source/payload/visible-row details are preserved in the JSON. No physical paint or matched T3 acceptance.'),
    ('Pin and unpin through local commit','web-pin-chat-mount1-2026-11-13','Default shortcut through real thread-store commit, final-row layout and two frames, with an unsent draft. Six retained samples per cell. Original message/composer nodes and focus are preserved in candidate runs; exact records, transcript and draft match. Partial client/action variants; all-action and T3 acceptance remain open.')]:
    lines += ['', '## '+title,'',scope,'','| History | Action | Implementation | Median ms | Range ms | Raw evidence |','| --- | --- | --- | ---: | ---: | --- |']
    for o in observations:
        if o['evidence_id']==evidence_id:
            lines.append(f"| {escape(o['workload'])} | {escape(o['operation'])} | {escape(o['variant'])} | {o['quantiles']['p50']:.3f} | {o['min']:.3f}–{o['max']:.3f} | [{evidence_id}]({link(o['raw_artifact'])}) |")

lines += ['', '## Resume command UI timings','',
          'Actual mounted web UI with existing synthetic history offscreen in Inbox. Native headless browser-memory Clipboard API; no OS Clipboard or physical paint. Six retained samples after one warmup per implementation/case/trigger. Menu open/filter, injected state setup, storage/history loading and verification are outside clocks. Primary rows end after Clipboard acknowledgement and two animation frames; no-write rows end after two frames. Invocation rows end at native writeText call. Zero readings are below timer resolution, not zero work. CPU work falls, while observed two-frame completion remains similar. No whole-action or matched T3 acceptance.','',
          '| Case | Trigger | Implementation | Clock | Median ms | Range ms | Raw evidence |',
          '| --- | --- | --- | --- | ---: | ---: | --- |']
for o in observations:
    if o['evidence_id']=='web-resume-export-ui1-2026-11-13':
        q=o['quantiles'];clock='Clipboard and two frames' if o['metric']=='elapsedMs' else 'Input to Clipboard call'
        if o['workload'] in {'no-active','no-session-75k'}:clock='Two frames; no write'
        lines.append(f"| {escape(o['workload'])} | {escape(o['operation'])} | {escape(o['variant'])} | {clock} | {q['p50']:.3f} | {o['min']:.3f}–{o['max']:.3f} | [{o['evidence_id']}]({link(o['raw_artifact'])}) |")
lines += ['', 'The [semantic action catalogue]('+link('docs/performance-action-catalogue.md')+') starts with 45 intents from 86 web/desktop registered commands. It does not establish downstream parity or complete application coverage. Working-tree search references now include callbacks after inline tests and global desktop app/dock code; comments/tests/lifecycle candidates remain unclassified.','']

lines += ['', 'The JSON companion retains all normalized cases, per-process quantiles, raw pointers, units and measurement scopes. Component decode timings and 128-event delivery diagnostics remain separate from the complete-delivery network rows.','',
          '## Static asset startup timings','',
          'Frozen ordinary release SPA with fixed empty services, real in-memory terminal and combined SSE, and mounted chat/command menu in desktop Chrome 157. Three retained intervals per cell after one warmup. Clock includes six startup reply bodies, three SSE handshakes, fonts and two frames after opening the menu. Fresh browser profile then same-context reload; OS filesystem warm. CDP impaired link is 1.6 Mbit/s / 300 ms. Physical mobile, radio, DNS/TLS, packet loss, populated histories and multi-environment catch-up are excluded. No matched T3 acceptance.','',
          '| Link and cache | Serving strategy | Samples | Median ms | Range ms | Raw evidence |',
          '| --- | --- | ---: | ---: | ---: | --- |']
for o in observations:
    if o['evidence_id']=='web-static-assets-network-boot1-2026-11-13':
        q=o['quantiles']
        lines.append(f"| {escape(o['workload'])} | {escape(o['variant'])} | {o['samples']} | {q['p50']:.3f} | {o['min']:.3f}–{o['max']:.3f} | [{o['evidence_id']}]({link(o['raw_artifact'])}) |")
lines += ['', 'Precompression costs about 14 ms at the local reload median and adds 5.31 MB in a web-resource-only ZIP model. Runtime gzip increases the local fresh-start median. Warm compressed WASM used browser cache; raw WASM transferred again. Three samples do not establish population tails. The separate completed-response byte aggregate extends through post-boundary validation and excludes open SSE. These remain partial startup variants; the full/signed update and equivalent T3 workloads are unmeasured.','',
          '## Terminal attach timings','',
          'Fixed, unmounted 1000 by 24 xterm parser with warm assets, exact 512 KiB replay and captured-state parity checks. The impaired connection runs at 5 Mbps with 120 ms round trip; direct runs use loopback. Timers end at the final xterm write callback. Paint, real mobile hardware, remote TCP connection setup, DNS and TLS are excluded. These remain partial action measurements; sample ranges do not establish population tail latency. Superseded prototypes and failed checks remain linked in the evidence record.','',
          '| Connection | Data | Version | Samples | Median ms | Range ms | WebSocket payload bytes | Raw evidence |',
          '| --- | --- | --- | ---: | ---: | ---: | ---: | --- |']
for o in observations:
    if o['id']=='GET /api/terminal/ws' and o.get('report_primary'):
        q=o['quantiles']
        lines.append(f"| {escape(o['group'])} | {escape(o['workload'])} | {escape(o['variant'])} | {o['samples']} | {q['p50']:.3f} | {o['min']:.3f}–{o['max']:.3f} | {o['websocket_payload_bytes']} | [{o['evidence_id']}]({link(o['raw_artifact'])}) |")
lines += ['', '## Conversation storage timings','',
          'Preliminary actual-WASM measurements with synthetic IndexedDB histories, warm connections and three retained observations per row after one warmup. Browser version was not captured. Main application mounting, UI/paint and provider/remote network work are excluded. Load-and-prepare ends after read-only loading, raw-original retention and display recovery. Append times only the conflict-checked local commit; setup, readback verification and restoration are outside that timer. This is not T3’s warm server-history read. Input byte counts describe the JSON representation, not physical disk or network bytes.','',
          '| Synthetic history | Operation | Samples | Median ms | Range ms | Input JSON bytes | Raw evidence |',
          '| --- | --- | ---: | ---: | ---: | ---: | --- |']
for o in observations:
    if o['evidence_id']=='web-history-storage-probe-2026-11-13' and o.get('report_primary') and o['measurement_level']=='local_storage_variant':
        q=o['quantiles']
        lines.append(f"| {escape(o['workload'])} | {escape(o['operation'])} | {o['samples']} | {q['p50']:.3f} | {o['min']:.3f}–{o['max']:.3f} | {o['data']['jsonBytes']} | [{o['evidence_id']}]({link(o['raw_artifact'])}) |")
lines += ['',
          'The JSON companion keeps codec component probes and correlated load/recovery stages separately. Zero stage readings are below the browser clock resolution, not zero work. The batch codec probe is not adopted: its direction varies by workload and operation. Full-record reads and writes still need separate bounded/incremental storage work.','',
          '## Conversation format comparisons','',
          'Warm actual-WASM storage measurements in private headless Chrome 157.0.8103.52. Each row has six retained intervals after one warmup, with counterbalanced implementation and case order. The full JSON envelope was rejected for large-output regressions; the hybrid format keeps small JSON headers and structured content/tool inputs. Converted reads improve, while untouched legacy records and structured fallback keep their existing paths. These are full-history local storage boundaries, not UI/paint, mobile, physical cold-I/O, network, server-send or T3 acceptance. Input bytes describe fixture JSON, not disk or wire bytes. Canonical hashes cannot distinguish non-finite values; the fallback fixture also checks exact float bits and typed commit/readback.','',
          '| Synthetic history | Storage boundary | Implementation | Samples | Median ms | Range ms | Raw evidence |',
          '| --- | --- | --- | ---: | ---: | ---: | --- |']
for o in observations:
    if o['evidence_id'] in {'web-conversation-history2-envelope-2026-11-13','web-conversation-history3-envelope-2026-11-13'}:
        q=o['quantiles']
        lines.append(f"| {escape(o['workload'])} | {escape(o['group'])} | {escape(o['variant'])} | {o['samples']} | {q['p50']:.3f} | {o['min']:.3f}–{o['max']:.3f} | [{o['evidence_id']}]({link(o['raw_artifact'])}) |")
lines += ['',
          'A formatted row means preparation through the actual writer; the fallback fixture remains structured. First legacy append includes conversion in the checked save. Setup, readback/restoration and wrapper diagnostics are outside the append timer. Small control/legacy/fallback differences overlap clock resolution or observed variation. Six samples do not establish population tails. Bounded history reads, incremental writes, full UI flows and the broader network/action audit remain required.','',
          '## Conversation save-queue completion','',
          'Actual optimized WASM TaskSaves edit through original-checked IndexedDB commit, acknowledgement and queue drain, including existing scheduler yields. The app is unmounted. Load/recovery, caller draft construction, verification, hashing and restoration are outside clocks. Six retained samples per cell after one warmup; case, format and implementation order counterbalanced. Prepared rows use the actual writer; non-finite fallback remains structured. Larger histories improve while short-history completion is mixed. Zero enqueue readings are below timer resolution. No whole-action, UI/paint, network/provider or matched T3 acceptance.','',
          '| Synthetic history | Format | Version | Median ms | Range ms | Raw evidence |','| --- | --- | --- | ---: | ---: | --- |']
for o in observations:
    if o['evidence_id']=='web-conversation-snapshot1-2026-11-13' and o['metric']=='drain_ms':
        q=o['quantiles']
        lines.append(f"| {escape(o['workload'])} | {escape(o['group'])} | {escape(o['variant'])} | {q['p50']:.3f} | {o['min']:.3f}–{o['max']:.3f} | [{o['evidence_id']}]({link(o['raw_artifact'])}) |")
lines += ['',
          '### Focused small-case replication','',
          'Same candidate and main binaries, with 32 retained observations per cell after one warmup. The candidate remains withheld: prepared structured fallback median completion is 9.8 ms on main and 26.3 ms on the candidate. Other short-history directions vary. The save loop yields again before noticing an empty queue; causation is unproven and requires a separate commit-boundary/scheduler investigation. All 396 intervals pass full readback, ordered counts and float checks.','',
          '| Synthetic history | Format | Version | Median ms | Range ms | Raw evidence |','| --- | --- | --- | ---: | ---: | --- |']
for o in observations:
    if o['evidence_id']=='web-conversation-snapshot1-small-control1-2026-11-13' and o['metric']=='drain_ms':
        q=o['quantiles']
        lines.append(f"| {escape(o['workload'])} | {escape(o['group'])} | {escape(o['variant'])} | {q['p50']:.3f} | {o['min']:.3f}–{o['max']:.3f} | [{o['evidence_id']}]({link(o['raw_artifact'])}) |")
lines += ['',
          '## Save commit and worker completion','',
          'New release-WASM studies separate the earliest IndexedDB complete event from acknowledgement and worker drain. The early observer registers when the native transaction is created, before Rust handlers; a late observer and frame ordinals check ordering. Existing Rust enqueue/drain times must match the three-clock bridge exactly. These are unmounted private local-store variants; caller draft preparation, loading/recovery, verification, restoration, physical UI and network/provider work are excluded.','',
          'Primary rows below use six retained observations after one warmup, natural entry timing and writer-prepared records. Non-finite fallback remains structured. Each successful candidate save schedules one frame; main schedules two. The complete legacy variants and five-clock distributions remain in JSON. Sample ranges are not population tails.','',
          '| History | Endpoint | Version | Median ms | Range ms | Raw evidence |','| --- | --- | --- | ---: | ---: | --- |']
for o in observations:
    if o['evidence_id']=='web-conversation-save2-2026-11-13' and o['group']=='formatted' and o['metric'] in {'commit_ms','drain_ms'}:
        q=o['quantiles'];lines.append(f"| {escape(o['workload'])} | {escape(o['metric'])} | {escape(o['variant'])} | {q['p50']:.3f} | {o['min']:.3f}–{o['max']:.3f} | [{o['evidence_id']}]({link(o['raw_artifact'])}) |")
lines += ['', '### Balanced requested entry phases','',
          'The independent focused cohort retains 32 samples per cell, eight at each requested fraction of a calibrated frame. Actual scheduling still varies; requested phase balance does not guarantee identical delivery. Keep the unfavorable entry-to-commit cells visible. After-yield short writes remain similar; worker completion improves. Phase setup is outside clocks. All 620 comparison saves pass complete readback/count/float, exact-clock and frame checks; 40 untimed controls pass. This establishes no whole-action or T3 acceptance.','',
          '| History | Endpoint | Version | Median ms | Range ms | Raw evidence |','| --- | --- | --- | ---: | ---: | --- |']
for o in observations:
    if o['evidence_id']=='web-conversation-save2-phase-control1-2026-11-13' and o['group']=='formatted' and o['metric'] in {'commit_ms','after_yield_commit_ms','drain_ms'}:
        q=o['quantiles'];lines.append(f"| {escape(o['workload'])} | {escape(o['metric'])} | {escape(o['variant'])} | {q['p50']:.3f} | {o['min']:.3f}–{o['max']:.3f} | [{o['evidence_id']}]({link(o['raw_artifact'])}) |")
lines += ['', 'The 204-interval diagnostic with the previous Arc-only candidate remains separate. The earlier late-listener experiment and capture-flag preflight are excluded as commit-boundary evidence. Source and observer corrections are retained with the study. Bounded history, actual UI/paint, provider/network and semantic all-action coverage remain open.','',
]

lines += ['', '## Direct original-envelope comparison (withheld)', '',
          'Published main (`53c1f1a3`) versus a strict direct-envelope check with typed fallback. Both versions keep one pre-write frame and the v12 format. The candidate is unmerged: long writer-prepared message lists have lower observed medians, while tool-heavy and legacy directions vary. Full histories are still read and rewritten. Six retained observations per cell after one warmup; 224 saves pass complete history/float/cache, exact-clock and event-order checks. Twenty-one codec controls and twenty functional saves pass. These unmounted local-store clocks exclude load/recovery/caller draft construction, verification/hash/restore, UI/paint, physical cold I/O, provider and network completion. Ranges are descriptive samples, not population tails.', '',
          '| History | Endpoint | Version | Median ms | Range ms | Raw evidence |',
          '| --- | --- | --- | ---: | ---: | --- |']
for o in observations:
    if o['evidence_id']=='web-conversation-envelope3-2026-11-13' and o['group']=='formatted' and o['metric'] in {'commit_ms','after_yield_commit_ms','drain_ms'}:
        q=o['quantiles'];lines.append(f"| {escape(o['workload'])} | {escape(o['metric'])} | {escape(o['variant'])} | {q['p50']:.3f} | {o['min']:.3f}–{o['max']:.3f} | [{o['evidence_id']}]({link(o['raw_artifact'])}) |")
lines += ['', 'The after_yield_commit_ms clock runs from the first pre-save scheduler callback through early transaction completion. commit_ms runs from edit_conversation entry through that same completion event. drain_ms runs from edit entry until the worker clears saving after acknowledgement. The latter two include initial scheduling. Legacy variants and all five clocks remain in JSON. Native differential testing found and fixed an empty-checkpoint normalization difference before timing. Ordinary release UI and a full merge gate remain required if this withheld candidate is reconsidered.', '']

lines += ['', '## Bounded conversation layout (withheld)', '',
          'One private optimized WASM binary compares full legacy reads/saves with indexed ranges and revision-checked row mutations. The one-message-per-row format and its schema upgrade remain unmerged: large-history pages and incremental mutations improve, but complete history reads and some small reads regress. A page is 77 messages, not T3 timeline items. There are 76 cells with six retained observations after one warmup, counterbalanced by case, operation and implementation. These local storage clocks exclude mounted UI, TaskSaves scheduling, physical cold I/O, providers and network.', '',
          'The raw runner records exit 1/pass=false because its final assertion expected 546 intervals. A separate exact-matrix audit accepts all 532 actual intervals: both the 2-message and 50-message fixtures lack earlier pages. Original raw evidence is preserved. Counts, full content/float readback and transaction event order pass. The private full-read path does not audit rows outside the declared interval, so production corruption handling remains incomplete.', '',
          '| History | Storage operation | Version | Median ms | Range ms | Raw evidence |',
          '| --- | --- | --- | ---: | ---: | --- |']
for o in observations:
    if o['evidence_id']=='web-conversation-bounded3-2026-11-13' and o['measurement_level']=='local_storage_variant':
        q=o['quantiles'];lines.append(f"| {escape(o['workload'])} | {escape(o['operation'])} | {escape(o['variant'])} | {q['p50']:.3f} | {o['min']:.3f}–{o['max']:.3f} | [{o['evidence_id']}]({link(o['raw_artifact'])}) |")
lines += ['', '### Initial indexed preparation (single intervals)', '',
          'One warm complete sidecar preparation per history, retaining the original legacy blob. These eight observations are outside repeated read/write clocks and are not repeated distributions. The 75,000-message preparation costs about 1.95 seconds. Indexed revision checks require every official writer and reader to migrate together behind a version barrier; web storage changes also leave native desktop persistence work open.', '',
          '| History | Messages | Preparation ms (n=1) | Raw evidence |',
          '| --- | ---: | ---: | --- |']
for o in observations:
    if o['evidence_id']=='web-conversation-bounded3-2026-11-13' and o['measurement_level']=='local_storage_setup_single_interval':
        lines.append(f"| {escape(o['workload'])} | {o['data']['messages']} | {o['quantiles']['p50']:.3f} | [{o['evidence_id']}]({link(o['raw_artifact'])}) |")

lines += ['', '## Fixed message-count chunk sweep (not adopted)', '',
          'One optimized private WASM binary compares whole-record storage with 1, 16, 128 and 1,024 messages per indexed row. All 760 intervals pass; 190 cells have three retained samples after one warmup, with alternating order. This is an exploratory grid, not a selected production budget. Large chunks recover full reads but erase mutation gains on content-heavy histories; small chunks retain bounded mutation work but add full-read overhead. All unfavorable cells remain below.', '',
          'Read clocks include request/decode/strict range and cardinality work, owned projection and terminal transaction completion. Complete snapshots also count all indexed thread rows, rejecting unexpected rows outside the range. Mutation clocks include encoding and parent/revision/affected-original checks through chunk/head writes and commit. Expected loading and caller draft construction precede clocks; result marshaling/hash/verification/restore, mounted UI, TaskSaves scheduling, physical cold I/O and providers/network are excluded. Per-record IDs and cross-size content/float parity pass. The earlier ID-comparison failure is preserved separately; no failed data is promoted.', '',
          '| History | Operation | Version | Median ms | Range ms | Raw evidence |',
          '| --- | --- | --- | ---: | ---: | --- |']
for o in observations:
    if o['evidence_id']=='web-conversation-chunk5-2026-11-13' and o['measurement_level']=='local_storage_variant':
        lines.append(f"| {escape(o['workload'])} | {escape(o['operation'])} | {escape(o['variant'])} | {o['quantiles']['p50']:.3f} | {o['min']:.3f}–{o['max']:.3f} | [{o['evidence_id']}]({link(o['raw_artifact'])}) |")
lines += ['', '### Chunk preparation (n=1 per case and size)', '',
          'Thirty-two single warm preparations retain the legacy blob and are outside repeated operation clocks. They are not repeated migration distributions. No format/schema or application reader/writer from this experiment has merged. Native persistence, complete consumers/recovery/version barriers and full UI/network/T3 acceptance remain open.', '',
          '| History | Version | Preparation ms (n=1) | Raw evidence |',
          '| --- | --- | ---: | --- |']
for o in observations:
    if o['evidence_id']=='web-conversation-chunk5-2026-11-13' and o['measurement_level']=='local_storage_setup_single_interval':
        lines.append(f"| {escape(o['workload'])} | {escape(o['variant'])} | {o['quantiles']['p50']:.3f} | [{o['evidence_id']}]({link(o['raw_artifact'])}) |")
lines += ['', '## Controlled task-stream request counts', '',
          'The task UI now reacts to complete refresh events, ignores keepalives/unrelated events, coalesces events within a chunk and invalidates pending replies on disconnect. This fix is applied to unpublished working-tree task-workspace code and is unmerged. The exact HTML passes 508 parser controls, 32 browser stream cases, 21 behavior checks, nine adversarial checks and two disconnect regressions. Intentional invalid-token 403s are recorded separately; the final browser gate has zero unexpected errors.', '',
          'In the earlier controlled-fragmentation study, real loopback HTTP returns one synthetic specification and no tasks; a browser ReadableStream controls SSE fragmentation. The eight matched remote desktop stream cases trigger 111 baseline snapshot requests and five candidate requests. Narrow-viewport and local-listener candidate cases also pass. These are request/body-byte counts from single controlled cases. That study does not measure latency, production wire bytes, real-runtime/TCP SSE or mobile/T3 acceptance. The separate actual-runtime study below keeps its own source, data and clocks.', '',
          '| Listener | Width px | Input | Version | Chunks | Snapshot requests | JSON body bytes |',
          '| --- | ---: | --- | --- | ---: | ---: | ---: |']
for o in request_counts:
    if o.get('raw_profile')=='real_runtime_sse':continue
    lines.append(f"| {'local' if o['local_listener'] else 'paired'} | {o['viewport_width']} | {escape(o['workload'])} | {o['variant']} | {o['source_chunks']} | {o['snapshot_requests']} | {o['response_body_bytes']} |")

runtime_observations=[o for o in observations if o['evidence_id']=='web-task-runtime6-2026-11-13']
if runtime_observations:
    lines += ['', '## Real task runtime and network timings', '',
              'Frozen optimized native CoordinationRuntime and paired-task remote_router, real TCP SSE/authentication/JSON persistence, four completed synthetic tasks with eight padded messages each and one specification. The before/after HTML pair preserves the same newer feature markup. No provider/background workers run. Small title/revision metadata differences remain visible in per-sample body sizes. This unpublished feature and its performance fix remain unmerged.', '',
              'Manual Refresh uses an in-page click through current DOM and two animation-frame callbacks; no physical pointer/paint claim. Three retained intervals follow one warmup per cell. Fetch body duration is the browser ResourceTiming boundary. The mobile profile is 390px, 1.6 Mbit/s down, 0.75 Mbit/s up and 300ms latency without CPU shaping, real device, TLS or proxy. Actual-runtime request suppression is verified; these samples do not establish a latency improvement or a matched T3 result. Exact response-body reductions belong to the separate controlled-fragmentation study.', '',
              '| Profile | Version | Boundary | Samples | Median ms | Range ms | Raw evidence |',
              '| --- | --- | --- | ---: | ---: | ---: | --- |']
    for o in runtime_observations:
        if o['operation']!='manual_refresh' or o['metric'] not in {'paint_ms','fetch_body_ms'}:continue
        lines.append(f"| {escape(o['workload'])} | {o['variant']} | {'Click through DOM and two frames' if o['metric']=='paint_ms' else 'Fetch through body'} | {o['samples']} | {o['quantiles']['p50']:.3f} | {o['min']:.3f}–{o['max']:.3f} | [{o['evidence_id']}]({link(o['raw_artifact'])}) |")
    lines += ['', 'A real Axum keepalive comment produces one snapshot GET before the fix and zero after it (one observed case per version). There is no production wire-byte total. Thirty-five controls pass, including owner/guest scoping, origin/token rejection, expiry, real SaveSpec persistence, draft/focus preservation and revoked-stream reader closure. Four separate notification diagnostics retain their single-interval arming/control-round-trip boundary in JSON and the linked report. The native child and all test browser contexts exit cleanly.', '',
              '| Version | Actual keepalive read chunks | Snapshot GETs | Raw evidence |',
              '| --- | ---: | ---: | --- |']
    for o in request_counts:
        if o.get('raw_profile')=='real_runtime_sse':lines.append(f"| {o['variant']} | {o['observed_read_chunks']} | {o['snapshot_requests']} | [Native runtime]({link(o['raw_artifact'])}) |")

pair_observations=[o for o in observations if o['evidence_id']=='web-task-pair1-2026-11-13']
if pair_observations:
    lines += ['', '## Fresh device pairing', '',
              'The unpublished task HTML now reuses its first authorized snapshot instead of downloading it again before showing tasks. The initial stream refresh remains to catch changes between snapshot and subscription. Pairing ownership is separate from snapshot-read ordering, so manual Refresh can finish during pairing and delayed old pairing replies cannot undo Disconnect or a newer attempt.', '',
              'The measured before/after HTML pair uses the same unchanged 88,264-byte snapshot from the frozen actual runtime used in the earlier study, with identical private forwarding adapters. The changing root backend is outside that build. Three retained samples follow one warmup per cell, in fresh browser sessions. Navigation and token entry precede the in-page form-submission clock. These clocks do not establish physical pointer/paint, whole-action or T3 equivalence. Loopback frame medians are higher in this small sample and ranges overlap; the impaired-link gain and traffic reduction do not imply a universal latency win.', '',
              '| Profile | Version | Endpoint | Median ms | Range ms | Raw evidence |',
              '| --- | --- | --- | ---: | ---: | --- |']
    for o in pair_observations:
        if not o['report_primary']:continue
        endpoint='First usable DOM and two frames' if o['metric']=='first_two_frames_ms' else 'Initial refresh settled and two frames'
        lines.append(f"| {escape(o['workload'])} | {o['variant']} | {endpoint} | {o['quantiles']['p50']:.3f} | {o['min']:.3f}–{o['max']:.3f} | [{o['evidence_id']}]({link(o['raw_artifact'])}) |")
    lines += ['', 'Every retained fresh-pairing trial uses three snapshot GETs before versus two after, with 264,792 versus 176,528 JSON body bytes. These exclude HTML navigation, HTTP/TCP overhead and the open SSE stream. Saved-token startup still uses two GETs. Independent browser verification passes 23 candidate checks, reproduces four expected baseline failures, and rejects unmatched errors. The existing local/guest project browser flow also passes. Source hashes, exact body parity and terminal process evidence are linked in the [pairing report]('+link('docs/performance-task-pairing-2026-11-13.md')+').', '']

compression=[o for o in observations if o['evidence_id']=='web-task-compression3-2026-11-13']
if compression:
    lines += ['', '## Snapshot transport compression', '',
              'The unmerged working-tree paired-task router is configured to negotiate gzip Fastest for successful finite responses whose uncompressed bodies are at least 768 bytes. The minimum comes from actual guest-proposal response controls: non-saving response TCP payload cases occurred through 512 bytes; 768 was the first tested size saving traffic in all four styles and three repetitions. These counts include HTTP headers and chunk framing, excluding IP/TCP headers and TLS. Seventy-two final-policy paired controls confirm unchanged traffic below the floor and positive savings in the tested larger cases. SSE/images/errors remain uncompressed; the optional favicon request receives an empty 204 response.', '',
              'The matrix below uses the earlier default 32-byte predicate, identity/Fastest/Default in one pinned optimized binary, five immutable payloads and the same task HTML. It has 480 retained native HTTP intervals plus 90 retained UI intervals, with 105 warmups. Native time includes complete encoded loopback HTTP, not isolated server CPU. UI ends after DOM update and two frames. Mobile is 390px, 1.6 Mbit/s down, 0.75 Mbit/s up and 300ms CDP latency, without CPU shaping. The 397-byte row is below the final policy floor and is prototype-only. These are not exact final-policy/full-current-root, physical-device/TLS/proxy, whole-action or matched T3 timings.', '',
              '| Payload | Transport | Encoded / raw body B | Native HTTP median ms (n=32) | Local UI median ms (n=3) | Mobile UI median ms (n=3) |',
              '| --- | --- | ---: | ---: | ---: | ---: |']
    for profile in ['zero00','small0','repeat','random','large0']:
        for variant in ['identity','fastest','default']:
            select=lambda metric,group:next(o for o in compression if o['workload']==profile and o['variant']==variant and o['metric']==metric and o['group']==group)
            n=select('elapsed_ms','native_loopback');local=select('paint_ms','loopback');mobile=select('paint_ms','mobile_1_6mbps_300ms')
            lines.append(f"| {profile}{' (below final floor)' if n['data']['below_final_policy_floor'] else ''} | {variant} | {n['data']['encoded_body_bytes']} / {n['data']['decoded_body_bytes']} | {n['quantiles']['p50']:.3f} | {local['quantiles']['p50']:.1f} | {mobile['quantiles']['p50']:.1f} |")
    lines += ['', 'Fastest was selected to limit native per-request work while retaining the main impaired-link gain. Default saved more bytes and was about 27ms faster for the largest repetitive UI fixture; it cost more native HTTP work. Small UI differences and overlapping local frame ranges do not prove a universal latency win. A 131-check prototype browser gate, 49 checks on the exact final router and 72 final byte controls pass. Original favicon failures and intermediate negative small-proposal results remain preserved. The router is tested over the older archived backend; the changing root, providers/concurrency and the full integration gate remain open. See the [compression report]('+link('docs/performance-task-compression-2026-11-13.md')+').', '']

if body_observations:
    lines += ['', '## Functional response-body observations', '',
              'Each row is one functional diagnostic per case/implementation. Fixture-server response writes match the recorded CDP received-body count; request/header/TCP/TLS overhead and controlled latency are excluded. Input bodies and completion contracts retain their source-specific scope in the linked reports. These rows are not timing samples.', '',
              '| Case | Implementation | Body bytes | Raw evidence |',
              '| --- | --- | ---: | --- |']
    for o in body_observations:
        lines.append(f"| {escape(o['case'])} | {escape(o['variant'])} | {o['response_body_bytes_written']:,} | [{escape(o['observation_id'])}]({link(o['raw_artifact'])}) |")

lines += ['',
          '## Discovered entrypoints','',
          'These identities overlap across surfaces and are discovery starting points. A view or binding is not automatically one complete user action. “Extraction pending” means older partial measurements exist but their numeric distributions have not yet been normalized into this log. Functional checks and request counts remain distinct from measured latency.','',
          '| Kind | Entrypoint | Source | Partial records | Timing status |','| --- | --- | --- | ---: | --- |']
for a in sorted(actions,key=lambda a:(a['kind'],a['id'])):
    path=link(a['source'])
    lines.append(f"| {a['kind']} | {escape(a['id'])} | [{Path(a['source']).name}:{a['line']}]({path}:{a['line']}) | {len(a['evidence_ids'])} | {a['timing_status']} |")
lines += ['','## Completion requirements','']+[f'{i+1}. {s}' for i,s in enumerate(result['remaining'])]
assert all(sha(p)==digest for p,(_,digest) in raw_cache.items()),'Timing source changed while rendering the log'
json_text=json.dumps(result,ensure_ascii=False,indent=2)+'\n'
markdown_text='\n'.join(lines)+'\n'
assert_publishable(json_text)
assert_publishable(markdown_text)
summary=make_summary(ROOT,ledger_path,ledger,result['counts']) if args.summary else None
(ROOT/args.json_path).write_text(json_text)
(ROOT/args.markdown).write_text(markdown_text)
if summary is not None:
    (ROOT/args.summary).write_text(json.dumps(summary,ensure_ascii=False,indent=2)+'\n')
print(json.dumps(result['counts']))
