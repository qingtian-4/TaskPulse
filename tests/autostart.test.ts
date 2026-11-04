import {test,expect} from 'bun:test';
import {get} from 'svelte/store';
import {createAutostartSettings,type AutostartStatus} from '../src/lib/autostart';

function deferred<T>() {
  let resolve!:(value:T)=>void;
  const promise=new Promise<T>(ok=>resolve=ok);
  return {promise,resolve};
}
const status=(enabled:boolean):AutostartStatus=>({enabled,available:true});

test('a successful startup setting survives read-only refreshes without extra writes',async()=>{
  let enabled=false,writes=0;
  const settings=createAutostartSettings({read:async()=>status(enabled),write:async value=>{writes++;enabled=value;return status(value);},settle:async()=>{}});
  await settings.refresh();
  await settings.setEnabled(true);
  await settings.refresh();
  expect(get(settings)).toMatchObject({status:status(true),pending:false,issue:null});
  expect(writes).toBe(1);
  await settings.setEnabled(false);
  expect(get(settings)).toMatchObject({status:status(false),pending:false,issue:null});
  expect(writes).toBe(2);
});

test('an asynchronously removed startup entry is reported and never written again',async()=>{
  let enabled=false,writes=0;
  const settings=createAutostartSettings({read:async()=>status(enabled),write:async value=>{enabled=value;writes++;return status(value);},settle:async()=>{enabled=false;}});
  await settings.refresh();await settings.setEnabled(true);
  expect(get(settings)).toMatchObject({status:status(false),pending:false,issue:'not-retained'});
  // Remounting General and window focus both call refresh on the same store.
  await settings.refresh();await settings.refresh();
  expect(get(settings).issue).toBe('not-retained');expect(writes).toBe(1);
  enabled=true;await settings.refresh();
  expect(get(settings).issue).toBe(null);expect(writes).toBe(1);
});

test('state removed after initial verification remains visible across tab navigation',async()=>{
  let enabled=false,writes=0;
  const settings=createAutostartSettings({read:async()=>status(enabled),write:async value=>{enabled=value;writes++;return status(value);},settle:async()=>{}});
  await settings.refresh();await settings.setEnabled(true);
  expect(get(settings).issue).toBe(null);
  enabled=false;await settings.refresh();await settings.refresh();
  expect(get(settings)).toMatchObject({status:status(false),issue:'not-retained'});expect(writes).toBe(1);
});

test('late pre-write reads and focus events cannot overwrite an in-flight enable',async()=>{
  const old=deferred<AutostartStatus>(),settled=deferred<void>();
  let reads=0,writes=0;
  const settings=createAutostartSettings({read:async()=>++reads===2?old.promise:status(reads>2),write:async()=>{writes++;return status(true);},settle:()=>settled.promise});
  await settings.refresh();const stale=settings.refresh();
  const save=settings.setEnabled(true);await settings.refresh();
  expect(get(settings).pending).toBe(true);expect(reads).toBe(2);
  old.resolve(status(false));await stale;
  expect(get(settings).status).toEqual(status(true));
  settled.resolve();await save;
  expect(get(settings)).toMatchObject({status:status(true),issue:null,pending:false});expect(writes).toBe(1);
});

test('blocked writes restore the actual checkbox state and preserve the error on focus',async()=>{
  const control={checked:true} as HTMLInputElement;
  let writes=0;
  const settings=createAutostartSettings({read:async()=>status(false),write:async()=>{writes++;throw Error('Access is denied.');},settle:async()=>{}});
  await settings.refresh();await settings.setEnabled(true,control);await settings.refresh();
  expect(control.checked).toBe(false);
  expect(get(settings)).toMatchObject({status:status(false),pending:false,issue:'write',detail:'Error: Access is denied.'});expect(writes).toBe(1);
});

test('mismatched backend results are errors and unavailable profiles never write',async()=>{
  let writes=0,failRead=false;
  const settings=createAutostartSettings({read:async()=>{if(failRead)throw Error('Read unavailable');return status(false);},write:async()=>{writes++;return status(false);},settle:async()=>{throw Error('must not settle an unsuccessful write');}});
  await settings.refresh();await settings.setEnabled(true);
  failRead=true;await settings.refresh();failRead=false;await settings.refresh();
  expect(get(settings)).toMatchObject({status:status(false),issue:'not-applied'});expect(writes).toBe(1);
  const isolated=createAutostartSettings({read:async()=>({enabled:false,available:false}),write:async()=>{throw Error('must not register isolated profiles');},settle:async()=>{}});
  await isolated.refresh();await isolated.setEnabled(true);
  expect(get(isolated)).toMatchObject({status:{enabled:false,available:false},issue:null,pending:false});
});

test('a transient read failure cannot erase a denied write after focus recovery',async()=>{
  let failRead=false,writes=0;
  const settings=createAutostartSettings({read:async()=>{if(failRead)throw Error('Read unavailable');return status(false);},write:async()=>{writes++;throw Error('Access is denied.');},settle:async()=>{}});
  await settings.refresh();await settings.setEnabled(true);
  failRead=true;await settings.refresh();
  failRead=false;await settings.refresh();
  expect(get(settings)).toMatchObject({issue:'write',detail:'Error: Access is denied.',status:status(false),pending:false});expect(writes).toBe(1);
});

test('verification failures are visible without reporting a confirmed success',async()=>{
  let reads=0;
  const settings=createAutostartSettings({read:async()=>{if(++reads>1)throw Error('Registry unavailable');return status(false);},write:async()=>status(true),settle:async()=>{}});
  await settings.refresh();await settings.setEnabled(true);
  expect(get(settings)).toMatchObject({pending:false,issue:'read',detail:'Error: Registry unavailable'});
});
