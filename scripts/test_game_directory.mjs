// Offline process/interoperability proof using a publicly known fixture key.
// Build server's game-directory example first; no live key or host is touched.
import assert from 'node:assert/strict';
import crypto from 'node:crypto';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import {spawnSync} from 'node:child_process';

const binary=process.argv[2];
assert(binary&&path.isAbsolute(binary),'pass the absolute game-directory example binary');
const root=fs.mkdtempSync(path.join(os.tmpdir(),'omoba-directory-proof-'));
const keyFile=path.join(root,'fixture-only.key'),input=path.join(root,'directory.json'),signed=path.join(root,'signed.json');
const seed=Buffer.alloc(32,7),rawSeed=seed.toString('hex'),checks=[];
const key=crypto.createPrivateKey({key:Buffer.concat([Buffer.from('302e020100300506032b657004220420','hex'),seed]),format:'der',type:'pkcs8'});
const publicKey=crypto.createPublicKey(key),publicHex=publicKey.export({type:'spki',format:'der'}).subarray(-32).toString('hex');
const execute=(args,ok)=>{
 const result=spawnSync(binary,args,{encoding:'utf8',timeout:5000,maxBuffer:100000});
 assert.ifError(result.error);assert(!result.stdout.includes(rawSeed)&&!result.stderr.includes(rawSeed),'key material leaked');
 assert.equal(result.status===0,ok,result.stderr);
 if(!ok)assert.equal(result.stdout,'','failed verification must not output a usable document');
 return result.stdout;
};
const verify=(file=signed,publicValue=publicHex,id='fixture',region='ru',now='150000',minimum='4')=>['verify',file,publicValue,id,region,'production',now,minimum];
try{
 const doc={version:1,region:'ru',contour:'production',sequence:4,issued_at_ms:100000,expires_at_ms:200000,
  policy:{min_release:'0.45.0',maintenance:true,update_url:null,message:'Проверка каталога'},lobbies:[]};
 fs.writeFileSync(input,JSON.stringify(doc));fs.writeFileSync(keyFile,rawSeed,{mode:0o600});
 const encoded=execute(['sign',input,keyFile,'fixture','ru','production','150000'],true);
 const envelope=JSON.parse(encoded);fs.writeFileSync(signed,encoded);
 const message=Buffer.concat([Buffer.from('omoba.game-directory.v1\0'),Buffer.from([Buffer.byteLength(envelope.key_id)]),Buffer.from(envelope.key_id),Buffer.from(envelope.payload)]);
 assert(crypto.verify(null,message,publicKey,Buffer.from(envelope.signature,'hex')));
 checks.push('native signature independently verified by Node Ed25519 over exact UTF-8 body/domain/key identity');
 assert.deepEqual(JSON.parse(execute(verify(),true)),doc);checks.push('actual sign/verify process round trip');
 execute(verify(signed,'0'.repeat(64)),false);checks.push('untrusted public key refused');
 execute(verify(signed,publicHex,'fixture','global'),false);checks.push('different account region refused');
 execute(verify(signed,publicHex,'fixture','ru','200000'),false);checks.push('expired document refused');
 execute(verify(signed,publicHex,'fixture','ru','150000','5'),false);checks.push('sequence rollback refused');
 const altered=path.join(root,'altered.json');
 fs.writeFileSync(altered,JSON.stringify({...envelope,payload:envelope.payload+' '}));execute(verify(altered),false);checks.push('one-byte payload mutation refused');
 fs.writeFileSync(altered,JSON.stringify({...envelope,key_id:'alias'}));execute(verify(altered,publicHex,'alias'),false);checks.push('key alias cannot reuse an old signature');
 if(process.platform!=='win32'){
  fs.chmodSync(keyFile,0o644);execute(['sign',input,keyFile,'fixture','ru','production','150000'],false);checks.push('unsafe key file permissions refused');
 }
}finally{fs.rmSync(root,{recursive:true});}
console.log(JSON.stringify({status:'PASS',scope:'offline synthetic fixture; no production key/network',checks,private_fixture_removed:!fs.existsSync(root)}));
