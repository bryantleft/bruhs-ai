"""Verify independent delivery checks with owned real HTTP/process/browser fixtures."""
from __future__ import annotations

import argparse
import http.client
import json
import math
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import sys
import tempfile
import traceback

from bench.workbench.run import ROOT, Campaign, digest
from bench.workbench.verification_review import save


GIT_PROXY = r"""import json, os, subprocess, sys
from pathlib import Path
args = sys.argv[1:]
command = args[:]
settings = []
while command[:1] == ['-c'] and len(command) >= 3:
    settings.append(command[1]); command = command[2:]
source = Path(os.environ['BRUHS_GUARD_SOURCE']).resolve()
private = Path(os.environ['BRUHS_GUARD_PRIVATE']).resolve()
cwd = Path.cwd().resolve()
inside_private = cwd.is_relative_to(private) and cwd != private
allowed_settings = all(setting in {'core.fsmonitor=false', 'core.quotePath=true', 'core.hooksPath=/dev/null', 'diff.autoRefreshIndex=false'} for setting in settings)
read_only = bool(command) and (
    command[0] in {'rev-parse','symbolic-ref','ls-files','ls-tree','diff','status','show','cat-file','rev-list','for-each-ref','check-attr'}
    or command[:2] == ['config','--get']
)
private_mutation = inside_private and bool(command) and (command[0] in {'checkout','add','write-tree','read-tree','update-index'} or command == ['remote','remove','origin'])
clone = command[:7] == ['clone','--shared','--no-checkout','--local','--no-tags','--',str(source)] and len(command) == 8
if clone:
    destination = Path(command[-1]).resolve()
    clone = cwd == source and destination.is_relative_to(private) and destination != private
allowed = allowed_settings and (read_only or private_mutation or clone)
with Path(os.environ['BRUHS_GUARD_GIT_LOG']).open('a') as handle:
    handle.write(json.dumps({'args':args,'command':command,'settings':settings,'cwd':str(cwd),'allowed':allowed,'inside_private':inside_private})+'\n')
if not allowed:
    print('Owned fixture blocked a mutating or unexpected Git command before execution.', file=sys.stderr)
    sys.exit(97)
sys.exit(subprocess.run([os.environ['BRUHS_GUARD_REAL_GIT'],*args],check=False).returncode)
"""

CHECK_FIXTURE = r"""import argparse, json, os, sys
from pathlib import Path
parser = argparse.ArgumentParser()
parser.add_argument('--log',required=True)
parser.add_argument('--phase',required=True)
args = parser.parse_args()
cwd = Path.cwd()
read = lambda name: (cwd/name).read_text() if (cwd/name).exists() else None
observed = {'cwd':str(cwd),'phase':args.phase,'selected':read('selected.txt'),'nested':read('nested/new.txt'),'tracked_unselected':read('tracked-unselected.txt'),'deleted_exists':(cwd/'removed.txt').exists(),'untracked_exists':(cwd/'unselected-private.txt').exists(),'ignored_exists':(cwd/'ignored/cache.txt').exists()}
with Path(args.log).open('a') as handle:
    handle.write(json.dumps(observed)+'\n');handle.flush();os.fsync(handle.fileno())
if args.phase == 'snapshot':
    assert observed['nested'] == 'NEW SELECTED\n', observed
    assert observed['tracked_unselected'] == 'HEAD ONLY\n', observed
    assert not observed['deleted_exists'] and not observed['untracked_exists'] and not observed['ignored_exists'], observed
    print('snapshot-only-selected: PASS')
elif args.phase == 'outcome':
    if observed['selected'] != 'GOOD\n':
        print('owned-check: FAIL exit 23',file=sys.stderr);sys.exit(23)
    print('owned-check: PASS')
elif args.phase == 'mutate':
    (cwd/'selected.txt').write_text('MUTATED PRIVATE SNAPSHOT\n')
    print('owned-check: source rewritten')
else:
    raise AssertionError(args.phase)
"""


def atomic_text(path: Path, value: str, mode: int | None = None) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile('w', dir=path.parent, delete=False) as handle:
        handle.write(value)
        handle.flush()
        os.fsync(handle.fileno())
        staged = Path(handle.name)
    if mode is not None:
        staged.chmod(mode)
    staged.replace(path)


def rows(path: Path) -> list[dict]:
    return [json.loads(line) for line in path.read_text().splitlines()] if path.exists() else []


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--server',type=Path,required=True)
    parser.add_argument('--output',type=Path,required=True)
    parser.add_argument('--timeout',type=float,default=45)
    parser.add_argument('--skip-browser',action='store_true')
    parser.add_argument('--legacy-diff-cache-guard',action='store_true',help='Only for an older server: disable optional index-stat refresh in the owned fixture repository.')
    args = parser.parse_args()
    if not math.isfinite(args.timeout) or args.timeout <= 0:
        parser.error('--timeout must be positive and finite')
    campaign = Campaign(args.server.resolve(),args.output,args.timeout)
    # Read-only Git probes must not refresh the owned original index cache.
    campaign.env['GIT_OPTIONAL_LOCKS'] = '0'
    web_assets = campaign.home/'web-assets'
    (web_assets/'fonts').mkdir(parents=True)
    shutil.copy2(ROOT/'bruhs-web/fonts/MonaspaceNeonVariable.woff2',web_assets/'fonts/MonaspaceNeonVariable.woff2')
    campaign.env['BRUHS_WEB_DIST'] = str(web_assets)
    for name in ('BRUHS_MAIL_DB','BRUHS_REACTIVE_DB','BRUHS_DELIVERY_DIR'):
        campaign.env[name] = str(campaign.home/name.lower())
    git = shutil.which('git')
    assert git
    git_log = campaign.output/'git-calls.jsonl'
    forge_log = campaign.output/'forge-calls.jsonl'
    check_log = campaign.output/'check-calls.jsonl'
    check_fixture = campaign.output/'fixture_checks.py'
    atomic_text(check_fixture,CHECK_FIXTURE)
    config = campaign.project/'.bruhs/config.json'
    files = ['selected.txt','nested/new.txt','removed.txt']
    commands = [dict(program=sys.executable,args=[str(check_fixture),'--log',str(check_log),'--phase',phase]) for phase in ('snapshot','outcome')]
    scripts = dict(setup=[],run=[],check=commands,teardown=[])

    def configure(checks: list[dict] = commands) -> None:
        save(config,dict(scripts,check=checks))

    def real_git(path: Path,*arguments: str) -> str:
        return subprocess.check_output([git,'-C',str(path),*arguments],env=campaign.env,text=True,stderr=subprocess.STDOUT).strip()

    atomic_text(campaign.project/'selected.txt','BASELINE\n')
    atomic_text(campaign.project/'tracked-unselected.txt','HEAD ONLY\n')
    atomic_text(campaign.project/'removed.txt','REMOVE ME\n')
    atomic_text(campaign.project/'.gitignore','ignored/\n')
    config.parent.mkdir()
    configure()
    real_git(campaign.project,'add','--all')
    real_git(campaign.project,'-c','user.name=fixture','-c','user.email=fixture@localhost','commit','-m','owned independent-check baseline')
    real_git(campaign.project,'config','remote.origin.url','https://github.com/owned-fixture/no-network.git')
    # Git diff can refresh stat-cache bytes even with optional locks disabled.
    # Keep exact index-byte accounting deterministic without changing its entries.
    if args.legacy_diff_cache_guard:
        real_git(campaign.project,'config','diff.autoRefreshIndex','false')
    other = campaign.home/'other-project'
    subprocess.run([git,'clone','--local','--no-hardlinks',str(campaign.project),str(other)],env=campaign.env,check=True,capture_output=True)
    real_git(other,'config','remote.origin.url','https://github.com/owned-fixture/no-network.git')
    atomic_text(other/'selected.txt','GOOD\n')
    atomic_text(other/'nested/new.txt','NEW SELECTED\n')
    (other/'removed.txt').unlink()
    atomic_text(campaign.project/'selected.txt','GOOD\n')
    atomic_text(campaign.project/'tracked-unselected.txt','PRIVATE ORIGINAL EDIT\n')
    atomic_text(campaign.project/'nested/new.txt','NEW SELECTED\n')
    atomic_text(campaign.project/'unselected-private.txt','UNSELECTED PRIVATE ORIGINAL\n')
    atomic_text(campaign.project/'ignored/cache.txt','IGNORED PRIVATE ORIGINAL\n')
    (campaign.project/'removed.txt').unlink()
    proxy = campaign.output/'fixture_git.py'
    atomic_text(proxy,GIT_PROXY)
    campaign.env.update(BRUHS_GUARD_REAL_GIT=git,BRUHS_GUARD_GIT_LOG=str(git_log),BRUHS_GUARD_SOURCE=str(campaign.project),BRUHS_GUARD_PRIVATE=str(campaign.home/'bruhs_delivery_dir/verification'))
    binary_dir = campaign.home/'bin'
    atomic_text(binary_dir/'git',f'#!/bin/sh\nexec {shlex.quote(sys.executable)} {shlex.quote(str(proxy))} "$@"\n',0o700)
    for program in ('gh','glab'):
        atomic_text(binary_dir/program,f'#!/bin/sh\nprintf "%s\\n" blocked >> {shlex.quote(str(forge_log))}\nexit 98\n',0o700)
    sources = [Path(__file__),ROOT/'bruhs-web/tests/delivery_verification_browser.mjs',ROOT/'bruhs-core/src/delivery.rs',ROOT/'bruhs-core/src/delivery/verification.rs',ROOT/'bruhs-core/src/delivery/plan.rs',ROOT/'bruhs-models/src/delivery.rs',ROOT/'bruhs-server/src/routes/delivery_ui.html',ROOT/'bruhs-web/fonts/MonaspaceNeonVariable.woff2']
    before = {str(path.relative_to(ROOT)):digest(path) for path in sources}
    for source in sources:
        destination = campaign.output/'source'/source.relative_to(ROOT)
        destination.parent.mkdir(parents=True,exist_ok=True)
        shutil.copy2(source,destination)
    report = {'scope':'Owned private HOME/repositories, real local server HTTP/process/store and headless browser. Original add/commit/push and all forge commands are blocked before execution; no models, credentials or live external writes. This is bounded acceptance evidence, not product completion.','server_sha256':digest(campaign.server),'source_sha256_before':before,'timeout_seconds':args.timeout,'fixture_disables_diff_cache':args.legacy_diff_cache_guard,'steps':[],'fatal_error':None,'cleanup_error':None,'passed':False}
    report_path = campaign.output/'report.json'
    responses: list[dict] = []

    def checkpoint(label: str,evidence: object) -> None:
        report['steps'].append({'label':label,'passed':True,'evidence':evidence})
        save(report_path,report)

    def api(method: str,path: str,body: dict | None = None,expected: int = 200) -> dict:
        connection = http.client.HTTPConnection('127.0.0.1',campaign.port,timeout=args.timeout)
        try:
            connection.request(method,path,json.dumps(body) if body is not None else None,{'Content-Type':'application/json'})
            response = connection.getresponse()
            raw = response.read().decode()
            value = json.loads(raw)
            responses.append({'method':method,'path':path,'status':response.status,'body':body,'response':value})
            save(campaign.output/'http-responses.json',responses)
            assert response.status == expected, responses[-1]
            return value
        finally:
            connection.close()

    def prepare(action: dict,path: Path = campaign.project,expected: int = 200) -> dict:
        return api('POST','/api/delivery/prepare',dict(forge='git_hub',path=str(path),timeout_seconds=30,action=action),expected)

    def deliver(proof: str | None,chosen: list[str] = files,path: Path = campaign.project,expected: int = 200) -> dict:
        action = dict(type='commit_push_pull',files=chosen,message='Owned reviewed payload',title='Owned reviewed payload',body='No external delivery is permitted by this fixture.',base='release',head='main',draft=True)
        if proof is not None:
            action['verification_id'] = proof
        return prepare(action,path,expected)

    def action(receipt_id: str) -> dict:
        return next(row for row in campaign.api('GET','/api/delivery')['actions'] if row['prepared']['id'] == receipt_id)

    def execute(prepared: dict) -> dict:
        api('POST','/api/delivery/execute',{'id':prepared['id']})
        settled = campaign.wait(lambda:(receipt if (receipt:=action(prepared['id']))['status'] not in {'prepared','running'} else None),'independent action settlement')
        save(campaign.output/f"receipt-{prepared['id']}.json",settled)
        return settled

    def original_state() -> dict:
        return dict(head=real_git(campaign.project,'rev-parse','HEAD'),index_sha256=digest(campaign.project/'.git/index'),tracked_unselected=(campaign.project/'tracked-unselected.txt').read_text(),untracked=(campaign.project/'unselected-private.txt').read_text(),ignored=(campaign.project/'ignored/cache.txt').read_text())

    initial = original_state()
    finished = False
    save(report_path,report)
    try:
        campaign.start()
        discovery = api('POST','/api/delivery/read',{'forge':'git_hub','path':str(campaign.project),'query':{'type':'scripts'}})
        assert discovery['scripts']['check'] == commands
        successful = prepare(dict(type='verify',files=files))
        assert successful['commands'] == commands and successful['workspace_path']
        assert not Path(successful['workspace_path']).exists() and not check_log.exists()
        assert action(successful['id'])['status'] == 'prepared'
        assert original_state() == initial
        checkpoint('Discovery and preparation freeze check commands without running them',{'prepared':successful,'check_process_count':0,'original':initial})
        configure([dict(program='/bin/sh',args=['-c','printf unexpected-live-config >&2; exit 66'])])
        completed = execute(successful)
        assert completed['status'] == 'completed' and completed['completed_commands'] == len(successful['commands']),completed
        assert completed['error'] is None and 'snapshot-only-selected: PASS' in completed['output'] and 'owned-check: PASS' in completed['output'],completed
        tree = completed.get('checked_tree')
        assert isinstance(tree,str) and len(tree) in (40,64) and all(char in '0123456789abcdefABCDEF' for char in tree),completed
        assert successful['worktree_fingerprint'].startswith('payload-v2:'),successful
        assert not Path(successful['workspace_path']).exists()
        observed = rows(check_log)
        assert len(observed) == len(successful['commands']) and all(row['cwd'] != str(campaign.project) and row['tracked_unselected'] == 'HEAD ONLY\n' and not row['untracked_exists'] and not row['ignored_exists'] and not row['deleted_exists'] for row in observed),observed
        configure()
        assert original_state() == initial
        checkpoint('Explicit execution runs frozen processes against HEAD plus selected payload only',{'receipt':completed,'observed':observed,'original_unchanged':True,'temporary_checkout_removed':True})

        atomic_text(campaign.project/'selected.txt','FAIL\n')
        failed = prepare(dict(type='verify',files=files))
        failed_receipt = execute(failed)
        assert failed_receipt['status'] == 'failed' and failed_receipt['completed_commands'] < len(failed['commands']) and '23' in failed_receipt['error'],failed_receipt
        rejected = deliver(failed['id'],expected=409)
        assert 'success' in rejected['error'].lower() or 'finish' in rejected['error'].lower(),rejected
        # A captured tree identifies a snapshot; only a completed receipt grants proof.
        atomic_text(campaign.project/'selected.txt','GOOD\n')
        checkpoint('A real failed process records exit 23 and cannot supply delivery evidence',{'receipt':failed_receipt,'rejected_preview':rejected})

        unproved = deliver(None)
        assert not unproved.get('verification')
        no_proof = execute(unproved)
        assert no_proof['status'] == 'failed' and no_proof['completed_commands'] == 0 and 'no independent check' in no_proof['error'].lower(),no_proof
        assert not [row for row in rows(git_log) if not row['allowed']] and not forge_log.exists()
        assert original_state() == initial
        checkpoint('Execution without check proof fails before any original Git or forge write',{'receipt':no_proof,'blocked_git_calls':[],'forge_calls':0,'original_unchanged':True})

        good_proposal = deliver(successful['id'],list(reversed(files)))
        proof = good_proposal['verification']
        assert proof['receipt_id'] == successful['id'] and proof['source_head'] == successful['expected_head'] and sorted(proof['files']) == sorted(files) and proof['commands'] == commands,proof
        assert proof['checked_tree'] == tree and proof['source_fingerprint'].startswith('payload-v2:'),proof
        mismatch_files = deliver(successful['id'],['selected.txt'],expected=409)
        mismatch_workspace = deliver(successful['id'],files,other,expected=409)
        assert 'different' in mismatch_files['error'].lower() and 'different' in mismatch_workspace['error'].lower()
        checkpoint('The host copies completed proof and binds it to HEAD, exact file selection and workspace',{'proposal':good_proposal,'different_files':mismatch_files,'different_workspace':mismatch_workspace})

        atomic_text(campaign.project/'selected.txt','STALE AFTER PASS\n')
        changed_preview = deliver(successful['id'],expected=409)
        assert 'source changed' in changed_preview['error'].lower(),changed_preview
        stale_execution = execute(good_proposal)
        assert stale_execution['status'] == 'failed' and stale_execution['completed_commands'] == 0 and 'changed' in stale_execution['error'].lower(),stale_execution
        assert not [row for row in rows(git_log) if not row['allowed']] and not forge_log.exists()
        atomic_text(campaign.project/'selected.txt','GOOD\n')
        checkpoint('Changed checked source is rejected at both preview and execution before Git writes',{'preview':changed_preview,'execution':stale_execution,'blocked_git_calls':[]})

        configure([dict(program=sys.executable,args=[str(check_fixture),'--log',str(check_log),'--phase','mutate'])])
        mutation = prepare(dict(type='verify',files=files))
        mutation_receipt = execute(mutation)
        assert mutation_receipt['status'] == 'failed' and 'changed the reviewed source snapshot' in mutation_receipt['error'],mutation_receipt
        assert (campaign.project/'selected.txt').read_text() == 'GOOD\n' and original_state() == initial
        configure()
        checkpoint('A zero-exit command that rewrites its private checked source cannot pass',{'receipt':mutation_receipt,'original_unchanged':True})

        pending = prepare(dict(type='verify',files=files))
        count_before = len(rows(check_log))
        campaign.stop()
        campaign.start()
        restored = action(successful['id'])
        pending_restored = action(pending['id'])
        assert restored == completed and pending_restored['status'] == 'prepared' and len(rows(check_log)) == count_before
        post_restart = deliver(successful['id'])
        assert post_restart['verification'] == proof
        checkpoint('Completed proof and prepared reviews persist across restart without replaying checks',{'completed':restored,'pending':pending_restored,'process_count_before':count_before,'process_count_after':len(rows(check_log)),'restored_proof':post_restart['verification']})
        save(campaign.output/'browser-input.json',{'base':f'http://127.0.0.1:{campaign.port}','project':str(campaign.project),'other_project':str(other),'files':files,'check_log':str(check_log),'git_log':str(git_log),'forge_log':str(forge_log),'timeout_ms':int(args.timeout*1000),'successful_id':successful['id'],'pending_id':pending['id'],'commands':commands})
        if not args.skip_browser:
            with (campaign.output/'browser.log').open('w') as log:
                browser_result = subprocess.run(['node',str(ROOT/'bruhs-web/tests/delivery_verification_browser.mjs'),str(campaign.output/'browser-input.json'),str(campaign.output)],stdout=log,stderr=subprocess.STDOUT,check=False)
            browser = json.loads((campaign.output/'browser-report.json').read_text())
            report['steps'].append({'label':'Real shared Delivery UI at desktop and mobile/touch widths','passed':browser_result.returncode==0 and browser['passed'],'evidence':browser})
            save(report_path,report)
        finished = True
    except (AssertionError,OSError,RuntimeError,ValueError,KeyError,StopIteration,subprocess.SubprocessError):
        report['fatal_error'] = traceback.format_exc()
    finally:
        try:
            campaign.stop()
        except (AssertionError,OSError,RuntimeError,ValueError,subprocess.SubprocessError):
            report['cleanup_error'] = traceback.format_exc()
        campaign.server_log.close()
        after = {str(path.relative_to(ROOT)):digest(path) for path in sources}
        report['source_sha256_after'] = after
        report['qa_harness_unchanged'] = all(before[str(path.relative_to(ROOT))] == after[str(path.relative_to(ROOT))] for path in sources[:2])
        report['concurrent_production_source_changes'] = [name for name in before if before[name] != after[name]]
        report['source_binding_note'] = 'Immutable binary is copied and hashed. Product source hashes are observations of a shared dirty workspace and may change concurrently; the parent owns the preceding build binding.'
        report['server_unchanged'] = digest(campaign.server) == report['server_sha256']
        report['git_calls'] = rows(git_log)
        report['blocked_git_calls'] = [row for row in report['git_calls'] if not row['allowed']]
        report['forge_calls'] = forge_log.read_text().splitlines() if forge_log.exists() else []
        report['check_calls'] = rows(check_log)
        report['original_after'] = original_state()
        report['original_head_and_index_unchanged'] = report['original_after'] == initial
        report['passed'] = finished and all(step['passed'] for step in report['steps']) and report['fatal_error'] is None and report['cleanup_error'] is None and report['qa_harness_unchanged'] and report['server_unchanged'] and not report['blocked_git_calls'] and not report['forge_calls'] and report['original_head_and_index_unchanged']
        save(report_path,report)
    print(json.dumps({'passed':report['passed'],'report':str(report_path),'fatal_error':report['fatal_error']}))
    return 0 if report['passed'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
