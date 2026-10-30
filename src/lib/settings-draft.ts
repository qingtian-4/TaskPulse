import type {DashboardSettings} from './types';

// Preference broadcasts may refresh the editor's baseline, but must never apply
// source drafts. A real source change requires the user to resolve the conflict.
export function reconcileSourceDraft(draft:DashboardSettings, applied:DashboardSettings, incoming:DashboardSettings) {
  const sameSources=JSON.stringify(incoming.sources)===JSON.stringify(applied.sources)
    && incoming.onboarding_complete===applied.onboarding_complete;
  return sameSources
    ? {conflict:false,settings:{...structuredClone(incoming),sources:structuredClone(draft.sources)}}
    : {conflict:true,settings:draft};
}
