import { writable, derived, get } from 'svelte/store';
import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { DashboardSettings } from './types';

export interface Preferences {
  theme:'system'|'light'|'dark'; language:'system'|'zh-CN'|'en'; show_on_start:boolean;
  show_model:boolean; show_elapsed:boolean; panel_density:'comfortable'|'compact';
}
export const LEGACY_PREFERENCES:Preferences={theme:'dark',language:'zh-CN',show_on_start:false,show_model:true,show_elapsed:true,panel_density:'comfortable'};
export const appSettings=writable<DashboardSettings|null>(null);
export const preferences=derived(appSettings,s=>s?.preferences??LEGACY_PREFERENCES);
export const locale=derived(preferences,p=>p.language==='system'?(typeof navigator!=='undefined'&&navigator.language.toLowerCase().startsWith('zh')?'zh-CN':'en'):p.language);
export const preferencePending=writable(false), preferenceError=writable('');
export function preferenceValues(s:DashboardSettings){return {preferences:{...LEGACY_PREFERENCES,...s.preferences},default_view:s.default_view,recent_days:s.recent_days??7,notifications:s.notifications,sound:s.sound,notification_mode:s.notification_mode??'popup',auto_check_updates:s.auto_check_updates??true,auto_discover_local:s.auto_discover_local??false};}
export type PreferenceValues=ReturnType<typeof preferenceValues>;
export type PreferencePatch=Omit<Partial<PreferenceValues>,'preferences'>&{preferences?:Partial<Preferences>};
let queue:Promise<void>=Promise.resolve(), pending=0;
export function savePreference(patch:PreferencePatch,control?:HTMLInputElement|HTMLSelectElement):Promise<void>{
  // Native controls change before onchange runs. Keep the last confirmed value
  // visible until persistence succeeds; Svelte caches checked/value properties
  // and cannot restore an unchanged model value after a rejected write.
  const confirmed=get(appSettings);
  if(control && confirmed){
    const values=preferenceValues(confirmed);
    const changes=patch.preferences??patch;
    const key=Object.keys(changes)[0];
    const previous=patch.preferences
      ? values.preferences[key as keyof Preferences]
      : values[key as keyof PreferenceValues];
    if(control.type==='checkbox') (control as HTMLInputElement).checked=Boolean(previous);
    else if(control.type==='radio'){
      for(const radio of (control.getRootNode() as Document|ShadowRoot).querySelectorAll<HTMLInputElement>('input[type="radio"]')){
        if(radio.name===control.name)radio.checked=radio.value===String(previous);
      }
    }else control.value=String(previous);
  }
  pending++;preferencePending.set(true);
  const work=queue.then(async()=>{
    preferenceError.set('');
    const current=get(appSettings);if(!current)throw Error('Settings have not loaded');
    const expected=preferenceValues(current);
    const values={...expected,...patch,preferences:{...expected.preferences,...patch.preferences}};
    const next=await invoke<DashboardSettings>('save_preferences',{values,expected});
    appSettings.set(next);
  }).catch(async error=>{
    // Refresh the compare-and-swap baseline after an external configuration edit.
    // Local source drafts are reconciled by their own editor, never submitted here.
    try{appSettings.set(await invoke<DashboardSettings>('get_monitoring_settings'));}catch{/* Retain the last confirmed settings. */}
    preferenceError.set(String(error));throw error;
  }).finally(()=>{pending--;preferencePending.set(pending>0);});
  queue=work.catch(()=>{});return work;
}
export async function connectPreferences():Promise<()=>void>{
  if(!isTauri())return ()=>{};
  let active=true;
  const stop=await listen<DashboardSettings>('monitoring-settings-updated',event=>{if(active)appSettings.set(event.payload);});
  try{const settings=await invoke<DashboardSettings>('get_monitoring_settings');if(active&&!get(appSettings))appSettings.set(settings);}
  catch(error){stop();throw error;}
  const stopTheme=preferences.subscribe(p=>{
    const resolved=p.theme==='system'?(matchMedia('(prefers-color-scheme: dark)').matches?'dark':'light'):p.theme;
    document.documentElement.dataset.theme=resolved;
  });
  const stopLanguage=locale.subscribe(value=>{document.documentElement.lang=value;});
  const media=matchMedia('(prefers-color-scheme: dark)');
  const changed=()=>{if(get(preferences).theme==='system')document.documentElement.dataset.theme=media.matches?'dark':'light';};
  media.addEventListener('change',changed);
  return ()=>{active=false;stop();stopTheme();stopLanguage();media.removeEventListener('change',changed);};
}
