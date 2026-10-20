// MS-2 UAT pass 2: real screenshot capture + the genuine over-quota attempt
// (t1 quota: vcpu 2, workloads 3 — a third 1-vcpu workload trips the vcpu
// ceiling after ms2b+ms2c already hold 2).
const http = require('http');
const fs = require('fs');
const T0 = process.env.T0, T1 = process.env.T1, ADMIN = process.env.ADMIN;

function jreq(method, path, body) {
  return new Promise((resolve, reject) => {
    const r = http.request(
      { host: '127.0.0.1', port: 9334, path, method },
      (res) => {
        let d = '';
        res.on('data', (c) => (d += c));
        res.on('end', () => { try { resolve(JSON.parse(d)); } catch (e) { reject(e); } });
      }
    );
    r.on('error', reject);
    if (body) r.write(JSON.stringify(body));
    r.end();
  });
}

async function main() {
  const list = await jreq('GET', '/json/list');
  const page = list.find((t) => t.type === 'page');
  const WebSocket = require('/tmp/node_modules/ws');
  const ws = new WebSocket(page.webSocketDebuggerUrl);
  await new Promise((res) => ws.on('open', res));
  let id = 0;
  const pending = new Map();
  ws.on('message', (m) => {
    const msg = JSON.parse(m);
    if (msg.id && pending.has(msg.id)) { pending.get(msg.id)(msg); pending.delete(msg.id); }
  });
  const call = (method, params = {}) =>
    new Promise((res) => { const i = ++id; pending.set(i, res); ws.send(JSON.stringify({ id: i, method, params })); });

  const nav = async (url) => { await call('Page.navigate', { url }); await new Promise((r) => setTimeout(r, 1400)); };
  const text = async () =>
    (await call('Runtime.evaluate', { expression: 'document.body.innerText', returnByValue: true })).result.result.value;
  const shot = async (name) => {
    const r = await call('Page.captureScreenshot', { format: 'png' });
    fs.writeFileSync('/tmp/ms2-uat-shots/' + name, Buffer.from(r.result.data, 'base64'));
    console.log('shot: ' + name);
  };
  const setCookie = (v) => call('Network.setCookie', { name: 'porche_token', value: v, url: 'https://dev.laplane.net/' });
  const clearCookie = () => call('Network.deleteCookies', { name: 'porche_token', url: 'https://dev.laplane.net/' });

  // t1 session
  await clearCookie();
  await nav(`https://dev.laplane.net/porche/user/w?token=${T1}`);
  await shot('u04-t1-home-isolated.png');
  await nav('https://dev.laplane.net/porche/user/w/ms2a');
  await shot('u05-t1-cross-tenant-404.png');

  // genuine over-quota: t1 holds ms2b(1) + ms2c(1) = 2 vcpu; one more exceeds
  await nav('https://dev.laplane.net/porche/user/w/new');
  await call('Runtime.evaluate', {
    expression: `(function(){
      const set=(n,v)=>{const e=document.querySelector(\`[name="\${n}"]\`); if(e) e.value=v;};
      set('id','ms2d'); set('command','true'); set('vcpu','1');
      set('ram_mib','128'); set('volume_mib',''); set('ip6','fd42:1:1::24'); set('gw6','fd42:1:1::ff');
      document.querySelector('form').submit();
    })()`, returnByValue: true });
  await new Promise((r) => setTimeout(r, 1400));
  const overquota = await text();
  console.log('overquota page mentions quota:', overquota.includes('quota'));
  console.log(overquota.match(/quota: [^<\n]*/)?.[0] || 'NO QUOTA REJECTION SEEN');
  await shot('u06b-t1-overquota-rejected.png');

  // t0 detail (tenant row + topology caption)
  await clearCookie();
  await nav(`https://dev.laplane.net/porche/user/w?token=${T0}`);
  await nav('https://dev.laplane.net/porche/user/w/ms2a');
  await shot('u03b-t0-detail-converged.png');

  // admin fleet (both tenants)
  await clearCookie();
  await nav(`https://dev.laplane.net/porche/admin/a?token=${ADMIN}`);
  await shot('u07-admin-fleet-both-tenants.png');

  ws.close();
}
main().catch((e) => { console.error('FAIL', e); process.exit(1); });
