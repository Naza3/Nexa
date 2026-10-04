"""Map actual resolved Maven runtime archives to unchanged upstream notices."""
import hashlib,io,json,os,re,sys,zipfile,xml.etree.ElementTree as ET
from pathlib import Path
app=Path(__file__).resolve().parents[1];assets=app/'assets';upstream=assets/'upstream'
raw=json.load(open(sys.argv[1]));cache=Path(os.environ['GRADLE_USER_HOME'])/'caches/modules-2/files-2.1'
NS={'m':'http://maven.apache.org/POM/4.0.0'}
def sha(data):return hashlib.sha256(data).hexdigest()
def pom(group,name,version):
    files=list((cache/group/name/version).glob('*/*.pom'))
    assert len(files)==1,(group,name,version,'missing or ambiguous POM')
    data=files[0].read_bytes();root=ET.fromstring(data)
    licenses=[{'name':n.findtext('m:name',namespaces=NS),'url':n.findtext('m:url',namespaces=NS)} for n in root.findall('m:licenses/m:license',NS)]
    identity={'coordinate':f'{group}:{name}:{version}','sha256':sha(data),'licenses':licenses}
    if not licenses and group=='com.google.guava' and name=='listenablefuture':
        parent=root.find('m:parent',NS);assert parent is not None
        parent_id=[parent.findtext('m:'+k,namespaces=NS) for k in ['groupId','artifactId','version']]
        assert parent_id==['com.google.guava','guava-parent','26.0-android'];identity['parent']=pom(*parent_id)
        identity['licenses']=identity['parent']['licenses']
    return identity
inventory=[];texts={};seen=set()
def text_add(source,data):
    data.decode('utf-8');key=sha(data)
    texts.setdefault(key,{'source':source,'text':data.decode('utf-8'),'size_bytes':len(data)})
    return key
for artifact in raw['artifacts']:
    coordinate=artifact['coordinate'];file=Path(artifact['file']);data=file.read_bytes();assert sha(data)==artifact['sha256']
    identity=(coordinate,artifact['sha256'])
    if identity in seen:continue
    seen.add(identity);group,name,version=coordinate.split(':');p=pom(group,name,version)
    entries=[];nested=0
    def scan(blob,prefix):
        global nested
        with zipfile.ZipFile(io.BytesIO(blob)) as z:
            for item in z.infolist():
                if item.is_dir():continue
                basename=Path(item.filename).name
                if re.match(r'^(license|licence|notice|copying|copyright)([._-]|$)',basename,re.I):
                    b=z.read(item);assert len(b)<2_000_000
                    entries.append({'path':prefix+item.filename,'sha256':text_add(coordinate+'!'+prefix+item.filename,b),'size_bytes':len(b)})
                if item.filename.endswith('.jar'):
                    nested+=1;scan(z.read(item),prefix+item.filename+'!')
    scan(data,'')
    if group=='io.flutter':
        assert version=='1.0.0-'+(Path(os.environ['FLUTTER_ROOT'])/'bin/internal/engine.version').read_text().strip()
        sources=[upstream/'flutter-engine-LICENSE.txt',Path(os.environ['FLUTTER_ROOT'])/'bin/cache/pkg/sky_engine/LICENSE']
    else:
        assert any('apache' in ((v['name'] or '')+' '+(v['url'] or '')).lower() for v in p['licenses']),(coordinate,p)
        if group=='org.jetbrains' and name=='annotations':sources=[upstream/'java-annotations-23.0.0-LICENSE.txt']
        elif group=='org.jetbrains.kotlinx':sources=[upstream/'kotlinx-coroutines-1.7.1-LICENSE.txt']
        elif group=='com.getkeepsafe.relinker':sources=[upstream/'relinker-1.4.5-LICENSE.txt']
        elif group=='org.jspecify':sources=[upstream/'jspecify-1.0.0-LICENSE.txt']
        elif group=='org.jetbrains.kotlin':sources=[upstream/'kotlin-LICENSE.txt']
        else:sources=[upstream/'Apache-2.0.txt']
    mapped=[text_add('Flutter SDK bin/cache/pkg/sky_engine/LICENSE' if f.name=='LICENSE' else 'upstream/'+f.name,f.read_bytes()) for f in sources]
    inventory.append({'coordinate':coordinate,'archive':file.name,'artifact_sha256':artifact['sha256'],'pom':p,'license_text_sha256':mapped,'embedded_notices':entries,'nested_archives_scanned':nested})
body=['Maven releaseRuntimeClasspath: actual resolved arm64 runtime artifacts.\nNo archive LICENSE/NOTICE entries were dropped. Shared Apache terms are mapped per coordinate below.\n']
for item in inventory:
    body.append('\n'+item['coordinate']+'\n  artifact SHA256: '+item['artifact_sha256']+'\n  license text SHA256: '+', '.join(item['license_text_sha256'])+'\n')
for digest,text in texts.items():body.append('\n===== '+text['source']+' | SHA256 '+digest+' =====\n\n'+text['text'])
(assets/'maven-notices.txt').write_text(''.join(body))
(assets/'maven-inventory.json').write_text(json.dumps({'schema_version':1,'source':'Gradle releaseRuntimeClasspath Android arm64 resolvedArtifacts','artifacts':inventory,'texts':{k:{'source':v['source'],'size_bytes':v['size_bytes']} for k,v in texts.items()}},indent=2)+'\n')
print(json.dumps({'unique_runtime_archives':len(inventory),'embedded_notice_entries':sum(len(x['embedded_notices']) for x in inventory),'distinct_texts':len(texts)}))
