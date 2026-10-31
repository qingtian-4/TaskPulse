import {get, writable} from 'svelte/store';
import {invoke} from '@tauri-apps/api/core';

export interface AutostartStatus {enabled:boolean; available:boolean}
interface AutostartState {
  status:AutostartStatus|null;
  pending:boolean;
  issue:'not-retained'|'not-applied'|'write'|'read'|null;
  detail:string;
}
interface Backend {
  read:()=>Promise<AutostartStatus>;
  write:(enabled:boolean)=>Promise<AutostartStatus>;
  settle:()=>Promise<void>;
}

export function createAutostartSettings(backend:Backend) {
  const state=writable<AutostartState>({status:null,pending:false,issue:null,detail:''});
  // Keep the user's intent across settings tabs, separately from the OS result.
  // This is session-only feedback, never a preference that re-enables startup.
  let requested:boolean|null=null, generation=0;

  async function refresh() {
    if(get(state).pending)return;
    const ticket=++generation;
    try {
      const status=await backend.read();
      if(ticket!==generation)return;
      const current=get(state);
      const changed=status.available && requested!==null && status.enabled!==requested;
      state.set({...current,status,
        issue:current.issue==='write'?'write':changed?(current.issue==='not-applied'?'not-applied':'not-retained'):null,
        detail:current.issue==='write'?current.detail:''});
    } catch(error) {
      if(ticket===generation)state.update(s=>s.issue==='write'||s.issue==='not-applied'?s:{...s,issue:'read',detail:String(error)});
    }
  }

  async function setEnabled(enabled:boolean,control?:HTMLInputElement) {
    const previous=get(state);
    // The native checkbox toggles before onchange. Show the confirmed value
    // until persistence succeeds, including when the result is unchanged.
    if(control)control.checked=previous.status?.enabled??false;
    if(previous.pending || !previous.status?.available)return;
    ++generation;requested=enabled;
    state.set({...previous,pending:true,issue:null,detail:''});
    try {
      const status=await backend.write(enabled);
      state.set({status,pending:true,issue:status.available && status.enabled===enabled?null:'not-applied',detail:''});
      if(status.available && status.enabled===enabled) {
        // Some startup managers revert a successful registry write asynchronously.
        // Verify once after a short grace period; never retry the write.
        await backend.settle();
        try {
          const verified=await backend.read();
          state.set({status:verified,pending:true,issue:verified.available && verified.enabled===enabled?null:'not-retained',detail:''});
        } catch(error) {
          state.update(s=>({...s,issue:'read',detail:String(error)}));
        }
      }
    } catch(error) {
      let status=get(state).status;
      try {status=await backend.read();} catch {/* Keep the last known OS state. */}
      state.set({status,pending:true,issue:'write',detail:String(error)});
    } finally {
      state.update(s=>({...s,pending:false}));
      if(control)control.checked=get(state).status?.enabled??false;
    }
  }

  return {subscribe:state.subscribe,refresh,setEnabled};
}

export const autostart=createAutostartSettings({
  read:()=>invoke<AutostartStatus>('get_autostart'),
  write:enabled=>invoke<AutostartStatus>('set_autostart',{enabled}),
  settle:()=>new Promise(resolve=>setTimeout(resolve,1000))
});
