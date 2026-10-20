// MS-2 UAT driver: two tenants + admin through a real browser over the
// public HTTPS path (dev.laplane.net/porche/...). Screenshots to
// /tmp/ms2-uat-shots/. Raw CDP — snap Chromium is AppArmor-walled off the
// tooling profile (recorded in MS-1's user-test.md Method note).
const http = require('http');

function req(method, path, body) {
  return new Promise((resolve, reject) => {
    const r = http.request(
      { host: '127.0.0.1', port: 9334, path, method },
      (res) => {
        let d = '';
        res.on('data', (c) => (d += c));
        res.on('end', () => resolve(JSON.parse(d)));
      }
    );
    r.on('error', reject);
    if (body) r.write(JSON.stringify(body));
    r.end();
  });
}

async function connect() {
  const list = await req('GET', '/json/list');
  const page = list.find((t) => t.type === 'page');
  const ws = new (require('ws'))(page.webSocketDebuggerUrl);
  await new Promise((res) => ws.on('open', res));
  let id = 0;
  const pending = new Map();
  ws.on('message', (m) => {
    const msg = JSON.parse(m);
    if (msg.id && pending.has(msg.id)) {
      pending.get(msg.id)(msg);
      pending.delete(msg.id);
    }
  });
  const call = (method, params = {}) =>
    new Promise((res) => {
      const i = ++id;
      pending.set(i, res);
      ws.send(JSON.stringify({ id: i, method, params }));
    });
  return { call, ws, close: () => ws.close() };
}

async function main() {
  const cdp = await connect();
  const out = {};

  const nav = async (url) => {
    await cdp.call('Page.navigate', { url });
    await new Promise((r) => setTimeout(r, 1200));
  };
  const text = async () =>
    (await cdp.call('Runtime.evaluate', { expression: 'document.body.innerText', returnByValue: true }))
      .result.result.value;
  const shot = async (name) =>
    (await cdp.call('Page.captureScreenshot', { format: 'png' })).result && name;
  const setCookie = async (value) => {
    await cdp.call('Network.setCookie', {
      name: 'porche_token',
      value,
      url: 'https://dev.laplane.net/',
    });
  };
  const clearCookie = async () => {
    await cdp.call('Network.deleteCookies', {
      name: 'porche_token',
      url: 'https://dev.laplane.net/',
    });
  };

  // -- u01: unauthenticated user surface → 401 with PoC-auth marker
  await clearCookie();
  await nav('https://dev.laplane.net/porche/user/w');
  out.u01_401 = await text();
  await shot('u01-user-401.png');

  // -- u02: t0 login via ?token= → workloads home, own view
  await nav(`https://dev.laplane.net/porche/user/w?token=${T0}`);
  await new Promise((r) => setTimeout(r, 800));
  out.u02_t0_home = await text();
  await shot('u02-t0-home.png');

  // -- u03: t0 creates a workload (real create path)
  await nav('https://dev.laplane.net/porche/user/w/new');
  await cdp.call('Runtime.evaluate', {
    expression: `(function(){
      const set=(n,v)=>{const e=document.querySelector(\`[name="\${n}"]\`); if(e) e.value=v;};
      set('id','ms2a'); set('command','printf ms2-hello'); set('vcpu','1');
      set('ram_mib','128'); set('volume_mib','256'); set('ip6','fd42:1:1::21'); set('gw6','fd42:1:1::ff');
      document.querySelector('form').submit();
    })()`,
    returnByValue: true,
  });
  await new Promise((r) => setTimeout(r, 1500));
  out.u03_t0_create = await text();
  await shot('u03-t0-create-detail.png');
  // let it converge, then check state
  await new Promise((r) => setTimeout(r, 4000));
  await nav('https://dev.laplane.net/porche/user/w/ms2a');
  out.u03_t0_detail = await text();
  await shot('u03b-t0-detail-converged.png');

  // -- u04: t1 session (fresh cookie) — sees ONLY its own (empty) view
  await clearCookie();
  await nav(`https://dev.laplane.net/porche/user/w?token=${T1}`);
  await new Promise((r) => setTimeout(r, 800));
  out.u04_t1_home = await text();
  await shot('u04-t1-home-isolated.png');

  // -- u05: t1 tries t0's workload detail → 404 (isolation, visual)
  await nav('https://dev.laplane.net/porche/user/w/ms2a');
  out.u05_t1_foreign_detail = await text();
  await shot('u05-t1-cross-tenant-404.png');

  // -- u06: t1 creates within quota, then exceeds it (quota: vcpu 2)
  await nav('https://dev.laplane.net/porche/user/w/new');
  await cdp.call('Runtime.evaluate', {
    expression: `(function(){
      const set=(n,v)=>{const e=document.querySelector(\`[name="\${n}"]\`); if(e) e.value=v;};
      set('id','ms2b'); set('command','sleep 60'); set('vcpu','1');
      set('ram_mib','128'); set('volume_mib',''); set('ip6','fd42:1:1::22'); set('gw6','fd42:1:1::ff');
      document.querySelector('form').submit();
    })()`,
    returnByValue: true,
  });
  await new Promise((r) => setTimeout(r, 1500));
  out.u06_t1_create = await text();
  await shot('u06-t1-create.png');
  // now over quota (t1 quota vcpu=2; 1+1 more exceeds)
  await nav('https://dev.laplane.net/porche/user/w/new');
  await cdp.call('Runtime.evaluate', {
    expression: `(function(){
      const set=(n,v)=>{const e=document.querySelector(\`[name="\${n}"]\`); if(e) e.value=v;};
      set('id','ms2c'); set('command','true'); set('vcpu','1');
      set('ram_mib','128'); set('volume_mib',''); set('ip6','fd42:1:1::23'); set('gw6','fd42:1:1::ff');
      document.querySelector('form').submit();
    })()`,
    returnByValue: true,
  });
  await new Promise((r) => setTimeout(r, 1200));
  out.u06b_t1_overquota = await text();
  await shot('u06b-t1-overquota-rejected.png');

  // -- u07: admin surface sees BOTH tenants' rows
  await clearCookie();
  await nav(`https://dev.laplane.net/porche/admin/a?token=${ADMIN}`);
  await new Promise((r) => setTimeout(r, 800));
  out.u07_admin_fleet = await text();
  await shot('u07-admin-fleet-both-tenants.png');

  // -- u08: tenant token on admin → 401 (domain split through tenancy)
  await clearCookie();
  await nav(`https://dev.laplane.net/porche/admin/a?token=${T1}`);
  await new Promise((r) => setTimeout(r, 800));
  out.u08_t1_on_admin = await text();
  await shot('u08-tenant-on-admin-401.png');

  console.log(JSON.stringify(out, null, 2));
  cdp.close();
}

main().catch((e) => {
  console.error('FAIL', e);
  process.exit(1);
});
