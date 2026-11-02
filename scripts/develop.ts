/** Official Tauri dev lifecycle: Vite HMR + Rust rebuild, no installer. */
import {createHash} from 'node:crypto';
import {realpathSync} from 'node:fs';
import {resolve,dirname,basename,delimiter} from 'node:path';
import {createServer} from 'node:net';

export function developmentConfig(root:string, port:number) {
  if(!Number.isInteger(port)||port<1024||port>65535)throw Error('Invalid development port');
  const id=createHash('sha256').update(process.platform==='win32'?root.toLowerCase():root).digest('hex').slice(0,12);
  const binary=basename(process.execPath);
  if(!/^[a-zA-Z0-9_.-]+$/.test(binary))throw Error('Unsupported Bun executable name');
  return {productName:'TaskPulse Dev',identifier:`app.taskpulse.dev.w${id}`,bundle:{active:false},
    build:{devUrl:`http://127.0.0.1:${port}`,beforeDevCommand:{script:`${binary} run dev`,cwd:root}},
    app:{security:{devCsp:`default-src 'self'; connect-src 'self' ipc: http://ipc.localhost http://127.0.0.1:${port} ws://127.0.0.1:${port}; style-src 'self' 'unsafe-inline'; img-src 'self' data:`}}};
}
if(import.meta.main){
  const root=realpathSync(resolve(import.meta.dir,'..'));
  const server=createServer();await new Promise<void>((ok,fail)=>{server.once('error',fail);server.listen(0,'127.0.0.1',ok);});
  const port=(server.address() as {port:number}).port;await new Promise<void>(ok=>server.close(()=>ok()));
  const args=['run','tauri','dev','--config',JSON.stringify(developmentConfig(root,port))];
  if(process.argv.slice(2).some(v=>v!=='--dry-run'))throw Error('Usage: bun run dev:desktop [--dry-run]');
  console.log(`TaskPulse Dev: http://127.0.0.1:${port}\nState: ~/.taskpulse-dev/<build-directory-id> (production data stays separate)`);
  if(process.argv.includes('--dry-run'))console.log(JSON.stringify({args,frontendPort:port},null,2));
  else {
    process.env.PATH=dirname(process.execPath)+delimiter+(process.env.PATH??'');
    process.env.TASKPULSE_DEV_PORT=String(port);
    process.env.CARGO_TARGET_DIR??=resolve(root,'src-tauri/target/desktop-dev');
    process.chdir(root);
    // Use the same in-process entry point as tauri.js. The official CLI owns
    // Ctrl+C handling and its Vite/cargo/app children, without an extra launcher.
    const {run}=await import('@tauri-apps/cli');
    await run(args.slice(2),'bun run dev:desktop');
  }
}
