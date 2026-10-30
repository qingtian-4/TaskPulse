import {test,expect,afterAll} from 'bun:test';
import {get} from 'svelte/store';
import {appSettings,savePreference,preferencePending,preferenceError,preferenceValues,LEGACY_PREFERENCES} from '../src/lib/preferences';
import {translate} from '../src/lib/i18n';
import {reconcileSourceDraft} from '../src/lib/settings-draft';
import type {DashboardSettings} from '../src/lib/types';
import english from '../locales/en.json';
import chinese from '../locales/zh-CN.json';

function fixture():DashboardSettings{return {version:2,onboarding_complete:true,preferences:{...LEGACY_PREFERENCES},notifications:true,sound:false,default_view:'tasks',recent_days:7,sources:[{id:'test-source',name:'用户名称',kind:'codex',enabled:false,location:'',projects:['protected-project'],excluded_projects:['private'],tmux_sessions:[],tmux_panes:[],poll_interval_ms:3000}]};}

test('locale catalogs agree and translate only explicit app copy',()=>{
  expect(Object.keys(english).sort()).toEqual(Object.keys(chinese).sort());
  expect(translate('通用','en')).toBe('General');
  expect(translate('发现新版本 {version}','en',{version:'1.2.3'})).toBe('Version 1.2.3 is available');
  expect(translate('用户自己的标题 / model-v2','en')).toBe('用户自己的标题 / model-v2');
});
test('preference broadcasts preserve source drafts and explicit scopes',()=>{
  const applied=fixture(),draft=structuredClone(applied),incoming=structuredClone(applied);
  draft.sources[0].projects=['unsaved-project'];incoming.preferences!.theme='light';
  const result=reconcileSourceDraft(draft,applied,incoming);
  expect(result.conflict).toBe(false);expect(result.settings.preferences!.theme).toBe('light');
  expect(result.settings.sources[0].projects).toEqual(['unsaved-project']);
  expect(incoming.sources[0].projects).toEqual(['protected-project']);
  incoming.sources[0].enabled=true;
  expect(reconcileSourceDraft(draft,applied,incoming).conflict).toBe(true);
});

const previousWindow=globalThis.window;
afterAll(()=>{Object.assign(globalThis,{window:previousWindow});appSettings.set(null);});
test('queued saves use the latest baseline, preserve sources, and recover after failure',async()=>{
  let saved=fixture(),fail=false;
  const calls:Array<{values:ReturnType<typeof preferenceValues>;expected:ReturnType<typeof preferenceValues>}>=[];
  Object.assign(globalThis,{window:{__TAURI_INTERNALS__:{invoke:async(command:string,args:any)=>{
    if(command==='get_monitoring_settings')return structuredClone(saved);
    expect(command).toBe('save_preferences');calls.push(structuredClone(args));
    await new Promise(resolve=>setTimeout(resolve,5));
    if(fail){fail=false;throw Error('disk write failed');}
    expect(args.expected).toEqual(preferenceValues(saved));
    saved={...saved,...args.values};return structuredClone(saved);
  }}}});
  appSettings.set(structuredClone(saved));
  const a=savePreference({preferences:{theme:'light'}}),b=savePreference({preferences:{language:'en'}});
  expect(get(preferencePending)).toBe(true);await Promise.all([a,b]);
  expect(calls[1].expected.preferences.theme).toBe('light');
  expect(saved.preferences!.language).toBe('en');expect(saved.sources).toEqual(fixture().sources);
  expect(calls.every(call=>!('sources' in call.values))).toBe(true);
  const failedControl={type:'checkbox',checked:false} as HTMLInputElement;
  fail=true;await expect(savePreference({preferences:{show_model:false}},failedControl)).rejects.toThrow('disk write failed');
  expect(failedControl.checked).toBe(true);
  expect(get(appSettings)!.preferences!.show_model).toBe(true);expect(get(preferencePending)).toBe(false);
  expect(get(preferenceError)).toContain('disk write failed');
  await savePreference({preferences:{show_elapsed:false}});
  expect(get(preferenceError)).toBe('');expect(saved.preferences!.show_elapsed).toBe(false);
});
