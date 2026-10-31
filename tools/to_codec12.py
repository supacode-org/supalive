# 開發用：把 Enhanced FLV（hvc1）改回抖音的 codec 12 格式，用來產生測試樣本。
# 用法：python3 tools/to_codec12.py in.flv out.flv
import sys
d=open(sys.argv[1],'rb').read(); out=bytearray(d[:13]); p=13
while p+11<=len(d):
    t=d[p]&31; n=int.from_bytes(d[p+1:p+4],'big'); body=d[p+11:p+11+n]
    if t==9 and body[0]&0x80:
        ft=(body[0]>>4)&7; pt=body[0]&15
        if pt==0: body=bytes([ft<<4|12,0,0,0,0])+body[5:]
        elif pt==1: body=bytes([ft<<4|12,1])+body[5:]
        elif pt==3: body=bytes([ft<<4|12,1,0,0,0])+body[5:]
        elif pt==2: body=bytes([ft<<4|12,2,0,0,0])
    out+=bytes([d[p]])+len(body).to_bytes(3,'big')+d[p+4:p+11]+body+(11+len(body)).to_bytes(4,'big')
    p+=11+n+4
open(sys.argv[2],'wb').write(out)
