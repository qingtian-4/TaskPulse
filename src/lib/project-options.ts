export interface ProjectChoice {id:string;path:string;aliases:string[]}

// Back-end aliases have been resolved inside the owning source. Preserve raw
// scope values, including configured patterns, instead of deduplicating names.
export function scopeOptions(projects:string[],choices:string[],known:ProjectChoice[]):ProjectChoice[] {
  const index=new Map<string,ProjectChoice>();
  for(const row of known)for(const value of [...row.aliases,row.path])index.set(value,row);
  const result=new Map<string,ProjectChoice>();
  for(const value of [...choices,...projects].filter(p=>p&&p!=='*')){
    const row=index.get(value)??{id:'literal:'+value,path:value,aliases:[value]};
    if(!result.has(row.id))result.set(row.id,{...row,aliases:[...new Set([...row.aliases,row.path])]});
  }
  return [...result.values()];
}
