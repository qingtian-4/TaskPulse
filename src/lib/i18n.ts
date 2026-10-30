import { derived, get } from 'svelte/store';
import { locale } from './preferences';
import english from '../../locales/en.json';
const dictionary:Record<string,string>=english;
export type TranslationValues=Record<string,string|number>;
export function translate(value:string, language:string=get(locale), values:TranslationValues={}):string {
  let text=language==='en'?(dictionary[value]??value):value;
  if(language==='en' && text===value){
    const age=value.match(/^(\d+) (秒|分钟|小时|天)前$/);
    if(age)text=age[1]+({秒:'s',分钟:'m',小时:'h',天:'d'} as Record<string,string>)[age[2]]+' ago';
  }
  return text.replace(/\{(\w+)\}/g,(token,key)=>String(values[key]??token));
}
export const t=derived(locale,language=>(value:string,values:TranslationValues={})=>translate(value,language,values));
