import hashlib, json, os, pathlib, plistlib, shutil, subprocess, tempfile
# macOS-only fixtures: all builds, signing and notarization are stubbed.
ROOT=pathlib.Path(__file__).resolve().parent.parent
VERSION=plistlib.loads((ROOT/'packaging/Info.plist').read_bytes())['CFBundleShortVersionString']
ROLLBACK_DATA = {
 'user-data/interface.json': b'{"theme":"dark"}',
 'user-data/assets/stamp.png': b'saved-stamp',
 'user-data/recovery/session.pdf': b'unsaved-document',
 'mypdf-user-data/workspace.json': b'{"tabs":["document.pdf"]}',
 'recovery/older-session.pdf': b'older-unsaved-document',
 'user-data-sha256.json': b'{"interface.json":"saved-original-hash"}',
}

def add_rollback_data(root):
 for relative, data in ROLLBACK_DATA.items():
  path=root/relative;path.parent.mkdir(parents=True,exist_ok=True);path.write_bytes(data)
 (root/'MyPDF.app').mkdir(exist_ok=True)
 (root/'MyPDF.app/old-binary').write_text('Do not archive obsolete binaries')

def assert_rollback_data(root):
 for relative, data in ROLLBACK_DATA.items():assert (root/relative).read_bytes()==data,(root,relative)

def assert_archive(root):
 assert_rollback_data(root)
 assert not (root/'MyPDF.app').exists()
 manifest=(root/'manifest.json').read_bytes()
 assert root.name=='rollback-data-'+hashlib.sha256(manifest).hexdigest()
 entries=json.loads(manifest)['entries']
 for relative, data in ROLLBACK_DATA.items():
  assert [relative,'file',hashlib.sha256(data).hexdigest()] in entries

STUB=r'''#!/usr/bin/env python3
import json, os, pathlib, plistlib, subprocess, sys
name=pathlib.Path(sys.argv[0]).name; args=sys.argv[1:]; cwd=pathlib.Path.cwd()
assert str(cwd).startswith(os.environ['FIXTURE_ROOT'])
with open(os.environ['STUB_LOG'],'a') as f:f.write(json.dumps([name,args])+"\n")
def arg(k):return args[args.index(k)+1]
def out(p,data='fixture'):
 p=pathlib.Path(p);p.parent.mkdir(parents=True,exist_ok=True);p.write_text(data)
if name=='uname': print('Darwin')
elif name=='rustc':print('rustc fixture\nhost: '+os.environ['STUB_HOST'])
elif name=='cargo':
 target=arg('--target') if '--target' in args else ''
 profile='release' if '--release' in args else 'debug'
 base=pathlib.Path(os.environ.get('CARGO_TARGET_DIR','target'))/target/profile
 if '--bin' in args:out(base/'mypdf','new-app-'+target)
 if '--example' in args:
  p=base/'examples/app_icon';out(p,'#!/bin/sh\nprintf fixture > "$1"\n');p.chmod(0o755)
elif name=='sips':out(arg('--out'))
elif name=='iconutil':out(arg('-o'))
elif name=='codesign':
 if args[-1].endswith('.app') and '--sign' in args:
  info=plistlib.loads((pathlib.Path(args[-1])/'Contents/Info.plist').read_bytes())
  expected=os.environ['EXPECT_DOCUMENT_HANDLERS']=='yes'
  assert ('CFBundleDocumentTypes' in info)==expected,(info,expected)
  if expected:
   assert any(t.get('LSHandlerRank')=='Alternate' and 'com.adobe.pdf' in t.get('LSItemContentTypes',[]) for t in info['CFBundleDocumentTypes'])
 if os.environ.get('FAIL_STAGE')=='sign-app' and '--options' in args:sys.exit(7)
 if os.environ.get('FAIL_STAGE')=='sign-dmg' and args[-1].endswith('.dmg'):sys.exit(7)
elif name=='ditto':
 if '-c' in args:out(args[-1])
 else:shutil=None;import shutil;shutil.copytree(args[-2],args[-1])
elif name=='xcrun':
 if args[:2]==['notarytool','submit']:
  rejected=os.environ.get('FAIL_STAGE')=='notary-dmg' and args[2].endswith('.dmg')
  print('{"status":"'+('Invalid' if rejected else 'Accepted')+'","id":"fixture"}')
 elif args[:2]==['stapler','staple'] and os.environ.get('FAIL_STAGE')=='staple-dmg' and args[-1].endswith('.dmg'):sys.exit(7)
elif name=='hdiutil':
 if os.environ.get('FAIL_STAGE')=='dmg':sys.exit(7)
 out(args[-1],'new-dmg')
elif name=='mv':
 if os.environ.get('FAIL_STAGE') in ['promote','restore'] and args[-1]=='dist/release' and args[-2].startswith('dist/.release-work.'):
  sys.exit(7)
 if os.environ.get('FAIL_STAGE')=='restore' and args==['dist/rollback','dist/release']:sys.exit(7)
 sys.exit(subprocess.run(['/bin/mv',*args]).returncode)
elif name=='spctl':pass
else:raise AssertionError(name)
'''
def run_case(label,host='aarch64-apple-darwin',failure=None,old=True,rollback=True,bundle=False,debug=False,archive_check=None,review=None,output=None):
 with tempfile.TemporaryDirectory(prefix='mypdf-packaging-test-') as raw:
  d=pathlib.Path(raw).resolve();(d/'scripts').mkdir();(d/'packaging').mkdir();(d/'bin').mkdir()
  for name in ['bundle-macos.sh','release-macos.sh']:shutil.copyfile(ROOT/'scripts'/name,d/'scripts'/name)
  shutil.copyfile(ROOT/'packaging/Info.plist',d/'packaging/Info.plist')
  stub=d/'bin/stub';stub.write_text(STUB);stub.chmod(0o755)
  for name in ['uname','rustc','cargo','sips','iconutil','codesign','ditto','xcrun','spctl','hdiutil','mv']:(d/'bin'/name).symlink_to('stub')
  for name,text in [('settings','settings-unchanged'),('stamps','stamps-unchanged'),('recovery','recovery-unchanged')]:
   (d/name).write_text(text)
  for path,text,enabled in [('dist/release','current-release',old),('dist/rollback','previous-rollback',rollback),('dist/MyPDF.app','dev-bundle',True)]:
   if enabled:(d/path).mkdir(parents=True);(d/path/'sentinel').write_text(text)
  if rollback:add_rollback_data(d/'dist/rollback')
  if failure=='archive-write':
   (d/'artifacts').mkdir();(d/'artifacts/preserved').write_text('blocked archive destination')
  if failure=='archive-symlink':
   (d/'do-not-copy').mkdir();(d/'do-not-copy/secret').write_text('outside snapshot')
   (d/'dist/rollback/user-data/external').symlink_to(d/'do-not-copy',target_is_directory=True)
  env={**os.environ,'PATH':str(d/'bin')+':'+os.environ['PATH'],'STUB_LOG':str(d/'commands.jsonl'),'STUB_HOST':host,'FIXTURE_ROOT':str(d),'MYPDF_SIGN_IDENTITY':'fixture-identity','MYPDF_NOTARY_PROFILE':'fixture-profile','CARGO_TARGET_DIR':'build-cache'}
  bundle_output=pathlib.Path(output or 'dist/MyPDF.app')
  handlers=not bundle or not (debug or review or bundle_output.name.lower().endswith('review.app'))
  env['EXPECT_DOCUMENT_HANDLERS']='yes' if handlers else 'no'
  if failure:env['FAIL_STAGE']=failure
  script='bundle-macos.sh' if bundle else 'release-macos.sh'
  args=['sh',str(d/'scripts'/script)]+(['--debug'] if debug else [])
  if review:args.append(review)
  if output:args+=['--output',output]
  p=subprocess.run(args,cwd=d,env=env,text=True,stdout=subprocess.PIPE,stderr=subprocess.STDOUT)
  assert (p.returncode!=0)==bool(failure),(label,p.returncode,p.stdout)
  commands=[json.loads(x) for x in (d/'commands.jsonl').read_text().splitlines()]
  builds=[a for n,a in commands if n=='cargo']
  expected=2 if host=='x86_64-apple-darwin' and not bundle else 1
  assert len(builds)==expected,(label,builds)
  if not bundle:
   assert builds[0].count('--bin')==1 and builds[0][builds[0].index('--target')+1]=='aarch64-apple-darwin',builds
   if expected==1:assert '--example' in builds[0]
   else:assert '--bin' not in builds[1] and builds[1][builds[1].index('--target')+1]==host
  archives=list((d/'artifacts/preserved').glob('rollback-data-*'))
  archive_expected=old and rollback and not bundle and failure in [None,'promote','restore']
  assert len(archives)==int(archive_expected),(label,archives,p.stdout)
  for archive in archives:assert_archive(archive)
  if failure=='restore':
   assert not (d/'dist/release').exists()
   assert (d/'dist/rollback/sentinel').read_text()=='current-release'
   candidates=list((d/'dist').glob('.release-work.*'));assert len(candidates)==1
   assert (candidates[0]/'previous-rollback/sentinel').read_text()=='previous-rollback'
   assert_rollback_data(candidates[0]/'previous-rollback')
   assert (candidates[0]/f'release/MyPDF-{VERSION}.dmg').exists()
   assert (d/'dist/.release-lock').exists()
   print('PASS',label)
   return
  if failure:
   if old:assert (d/'dist/release/sentinel').read_text()=='current-release',label
   if rollback:
    assert (d/'dist/rollback/sentinel').read_text()=='previous-rollback',label
    assert_rollback_data(d/'dist/rollback')
   if failure=='archive-symlink':
    assert (d/'do-not-copy/secret').read_text()=='outside snapshot'
    assert (d/'dist/rollback/user-data/external').is_symlink()
  elif bundle:
   assert (d/bundle_output/'Contents/MacOS/MyPDF').read_text()=='new-app-',label
   assert ('--release' in builds[0])!=debug
   actual=plistlib.loads((d/bundle_output/'Contents/Info.plist').read_bytes())
   expected=plistlib.loads((d/'packaging/Info.plist').read_bytes())
   if not handlers:expected.pop('CFBundleDocumentTypes',None)
   assert actual==expected,(label,actual,expected)
   # Packaging a review never edits the product plist or an existing release.
   assert (d/'packaging/Info.plist').read_bytes()==(ROOT/'packaging/Info.plist').read_bytes()
  else:
   assert (d/'dist/release/MyPDF.app/Contents/MacOS/MyPDF').read_text()=='new-app-aarch64-apple-darwin',label
   assert (d/f'dist/release/MyPDF-{VERSION}.dmg.sha256').is_file(),label
   if old:assert (d/'dist/rollback/sentinel').read_text()=='current-release',label
   elif rollback:
    assert (d/'dist/rollback/sentinel').read_text()=='previous-rollback',label
    assert_rollback_data(d/'dist/rollback')
   assert (d/'dist/MyPDF.app/sentinel').read_text()=='dev-bundle',label
  for name in ['settings','stamps','recovery']:assert (d/name).read_text()==name+'-unchanged'
  assert not list((d/'dist').glob('.release-*')),(label,list((d/'dist').iterdir()))
  assert not list((d/'dist').glob('.mypdf-bundle.*')),label
  if archive_check:
   # Retry the same old snapshot: reuse its verified archive without duplicating
   # data, and refuse promotion if a retained archive has since been damaged.
   shutil.rmtree(d/'dist/rollback')
   (d/'dist/rollback').mkdir();(d/'dist/rollback/sentinel').write_text('previous-rollback')
   add_rollback_data(d/'dist/rollback')
   if archive_check=='corrupt':(archives[0]/'user-data/interface.json').write_text('damaged archive')
   again=subprocess.run(args,cwd=d,env=env,text=True,stdout=subprocess.PIPE,stderr=subprocess.STDOUT)
   assert (again.returncode!=0)==(archive_check=='corrupt'),again.stdout
   assert list((d/'artifacts/preserved').glob('rollback-data-*'))==archives
   if archive_check=='corrupt':
    assert (d/'dist/rollback/sentinel').read_text()=='previous-rollback'
    assert_rollback_data(d/'dist/rollback')
   else:assert_archive(archives[0])
   assert (d/'dist/release/MyPDF.app/Contents/MacOS/MyPDF').read_text()=='new-app-aarch64-apple-darwin'
   assert not list((d/'dist').glob('.release-*'))
  print('PASS',label)
run_case('ARM64 release: one compile and rollback rotation')
run_case('Intel release: cross app plus host icon helper',host='x86_64-apple-darwin')
for fail in ['sign-app','sign-dmg','dmg','notary-dmg','staple-dmg','promote']:run_case('preserve current + rollback after '+fail,failure=fail)
run_case('first release',old=False,rollback=False)
run_case('retain existing rollback with no current release',old=False)
run_case('native bundle',bundle=True)
run_case('native debug bundle',bundle=True,debug=True)
run_case('optimized review bundle has no document handlers before signing',bundle=True,review='--review')
run_case('no-document-handlers alias',bundle=True,review='--no-document-handlers')
run_case('review output name excludes handlers case-insensitively',bundle=True,output='dist/MyPDF rEvIeW.app')
run_case('ordinary optimized custom bundle retains PDF handler',bundle=True,output='dist/custom/MyPDF.app')

run_case('preserve recovery files and lock when restoration fails',failure='restore')
run_case('archive failure preserves both releases and all user data',failure='archive-write')
run_case('user data symlinks stop publication without following them',failure='archive-symlink')
run_case('identical user data reuses its archive',archive_check='reuse')
run_case('damaged retained archive stops publication',archive_check='corrupt')
