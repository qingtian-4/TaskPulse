import type { DashboardSettings, MonitorSource, SourceKind, SourceDiscovery } from './types';

export type CapabilityEvidence='unsupported'|'explicit'|'inferred'|'hook'|'producer';
export interface SourceCapabilities {
  discovery:CapabilityEvidence; model:CapabilityEvidence; turn_start:CapabilityEvidence;
  turn_end:CapabilityEvidence; approval:CapabilityEvidence; question:CapabilityEvidence;
  return_target:'details'|'host_window';
}
export interface SourceDefinition {
  description?:string; id:string; name:string; kind:SourceKind; category:string;
  location_kind:string; platforms:string[]; adapter_id?:string; agent_id?:string;
  multiple_instances?:boolean; auto_enable_on_discovery?:boolean;
  capabilities?:SourceCapabilities; verified_platforms?:string[]; default_source?:MonitorSource;
}
// Use the same reviewable rows on the panel and on the settings landing page.
export function localDiscoveryCandidates(catalog:SourceDefinition[], found:SourceDiscovery[], settings:DashboardSettings|null){
  if(!settings?.onboarding_complete)return [];
  const seen=new Set(settings.auto_discovery_seen??[]);
  return catalog.filter(d=>d.category==='local'&&d.kind!=='scanner').flatMap(definition=>{
    if(settings.sources.some(s=>s.kind===definition.kind))return [];
    const row=found.find(d=>d.data_location_detected===true && d.candidate?.kind===definition.kind
      && !seen.has(d.candidate.id) && !settings.sources.some(s=>s.id===d.candidate?.id));
    return row?.candidate?[{definition,candidate:row.candidate}]:[];
  });
}
export function configuredSource(source:MonitorSource, definition:SourceDefinition|undefined, seen:string[]=[]):boolean {
  const seed=definition?.default_source;
  return source.enabled || seen.includes(source.id) || !seed || source.id!==seed.id
    || source.name!==seed.name || source.location!==seed.location || source.poll_interval_ms!==seed.poll_interval_ms
    || JSON.stringify(source.projects)!==JSON.stringify(seed.projects)
    || !!(source.excluded_projects.length||source.tmux_sessions.length||source.tmux_panes.length||source.integration);
}
