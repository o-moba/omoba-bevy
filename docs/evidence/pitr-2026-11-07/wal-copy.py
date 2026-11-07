import hashlib,os,pathlib,re,sys,uuid
mode,archive,name,path=sys.argv[1:];archive=pathlib.Path(archive);path=pathlib.Path(path)
if not re.fullmatch(r"[0-9A-F]{24}(?:\.[0-9A-F]{8}\.backup|\.partial)?|[0-9A-F]{8}\.history",name):raise SystemExit(2)
source,destination=(path,archive/name) if mode=="archive" else (archive/name,path)
if not source.is_file():raise SystemExit(1)
data=source.read_bytes()
if mode=="archive" and destination.exists():raise SystemExit(0 if destination.read_bytes()==data else 3)
temporary=destination.with_name(destination.name+".pending-"+uuid.uuid4().hex)
with temporary.open("xb") as stream:stream.write(data);stream.flush();os.fsync(stream.fileno())
try:
 if mode=="archive":os.link(temporary,destination);temporary.unlink()
 else:os.replace(temporary,destination)
finally:
 if temporary.exists():temporary.unlink()
fd=os.open(destination.parent,os.O_RDONLY);os.fsync(fd);os.close(fd)
