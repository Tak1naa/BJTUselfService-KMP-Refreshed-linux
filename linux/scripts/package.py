#!/usr/bin/env python3
"""Build local DEB/RPM packages; never install them or contact a release server."""
import argparse,gzip,json,shutil,subprocess,tarfile,tempfile,tomllib
from pathlib import Path
p=argparse.ArgumentParser();p.add_argument('binary',type=Path);p.add_argument('--format',choices=['deb','rpm','all'],default='all');p.add_argument('--output',type=Path,default=Path('artifacts/packages'));a=p.parse_args();binary=a.binary.resolve();out=a.output.resolve();out.mkdir(parents=True,exist_ok=True);root=Path(__file__).resolve().parents[1];version=tomllib.loads((root/'Cargo.toml').read_text())['workspace']['package']['version']
if subprocess.check_output([binary,'--version'],text=True).strip() != f'campus {version}':
    raise SystemExit('packaging binary version does not match Cargo.toml')
stage=Path(tempfile.mkdtemp(prefix='campus-package-',dir=out));(stage/'usr/bin').mkdir(parents=True);shutil.copy2(binary,stage/'usr/bin/campus')
for shell,path in [('bash','usr/share/bash-completion/completions/campus'),('zsh','usr/share/zsh/site-functions/_campus'),('fish','usr/share/fish/vendor_completions.d/campus.fish')]:
    target=stage/path;target.parent.mkdir(parents=True,exist_ok=True);target.write_bytes(subprocess.check_output([binary,'completions',shell]))
docs=stage/'usr/share/doc/campus-bjtu';docs.mkdir(parents=True);shutil.copy2(root/'README.md',docs/'README.md');shutil.copy2(root.parent/'LICENSE',docs/'copyright')
man=sorted(binary.parent.glob('build/campus-cli-*/out/campus.1'),key=lambda p:p.stat().st_mtime_ns)
if man:
    target=stage/'usr/share/man/man1/campus.1.gz';target.parent.mkdir(parents=True,exist_ok=True);target.write_bytes(gzip.compress(man[-1].read_bytes(),mtime=0))
elf=subprocess.check_output(['readelf','-l',str(binary)],text=True)
if 'INTERP' in elf:raise SystemExit('DEB/RPM packaging requires the static musl binary')
artifacts=[]
if a.format in ['deb','all']:
    meta=stage/'DEBIAN';meta.mkdir();(meta/'control').write_text(f'Package: campus-bjtu\nVersion: {version}\nArchitecture: amd64\nMaintainer: BJTU campus contributors\nSection: utils\nPriority: optional\nDescription: Offline-first BJTU Linux CLI\n')
    path=out/f'campus-bjtu_{version}_amd64.deb';subprocess.run(['dpkg-deb','--root-owner-group','--build',str(stage),str(path)],check=True);artifacts.append(str(path));shutil.rmtree(meta)
if a.format in ['rpm','all']:
    top=out/'rpmbuild'
    for d in ['BUILD','BUILDROOT','RPMS','SOURCES','SPECS','SRPMS','tmp']:(top/d).mkdir(parents=True,exist_ok=True)
    archive=top/'SOURCES/campus-files.tar.gz'
    with tarfile.open(archive,'w:gz')as tar:
        for f in sorted(stage.rglob('*')):tar.add(f,arcname=f.relative_to(stage),recursive=False)
    spec=(root/'packaging/campus.spec').read_text();specpath=top/'SPECS/campus.spec';specpath.write_text(spec)
    subprocess.run(['rpmbuild','-bb','--define',f'_topdir {top}','--define',f'_tmppath {top / "tmp"}',str(specpath)],check=True)
    for path in (top/'RPMS').rglob('*.rpm'):target=out/path.name;shutil.copy2(path,target);artifacts.append(str(target))
print(json.dumps({'packages':artifacts},indent=2))
