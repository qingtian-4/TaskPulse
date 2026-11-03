/** Convert an operator-certified, complete TaskPulse delivery log copy into
 * observations. Ground truth is authored before reading the TaskPulse log. */
import {readFileSync} from 'node:fs';
import {scoreRequests,type Measurement,type Measurements,type Truth} from './request-evaluation';

type Delivery = {at:number; stage:string; channel:string; notice_id:number|null; task_id:string|null; request_id?:string|null; waiting_reason?:string|null; test:boolean};
export interface Capture {
  from:number;
  until:number;
  complete:boolean;
  taskIds:string[];
  bindings:{id:string;taskId:string;requestId?:string;restored?:boolean;returned?:Measurement['returned']}[];
  resourceSamples?:Measurements['resourceSamples'];
}
export function joinDelivery(truth:Truth, log:{entries:Delivery[]}, capture:Capture):Measurements {
  if(truth.cohort.lane!=='real'||truth.oracle!=='host_observation')throw Error('Supply independently observed host truth');
  if(capture.complete!==true||!Number.isSafeInteger(capture.from)||!Number.isSafeInteger(capture.until)||capture.until<=capture.from)
    throw Error('Confirm an uninterrupted observation window');
  // The on-disk log retains at most 500 rows. At the cap we cannot establish
  // that a negative window did not lose an alert to eviction.
  if(!Array.isArray(log.entries)||log.entries.length>=500)throw Error('Use an untruncated delivery log copy');
  const scope=new Set(capture.taskIds);
  if(!scope.size||capture.bindings.length!==truth.cases.length||new Set(capture.bindings.map(b=>b.id)).size!==truth.cases.length)
    throw Error('Supply a unique binding for every truth case');
  const windows=truth.cases.map(t=>{
    const b=capture.bindings.find(x=>x.id===t.id);
    if(!b||!scope.has(b.taskId)||t.observableAt<capture.from||t.observeUntil>capture.until)
      throw Error('Missing binding or incomplete window');
    if(t.expected==='request'&&!b.requestId?.trim())throw Error('A positive binding needs an independently matched request ID');
    return {truth:t,b};
  });
  for(const [i,w] of windows.entries()){
    if(windows.slice(i+1).some(o=>o.b.taskId===w.b.taskId
      &&o.truth.observableAt<w.truth.observeUntil&&w.truth.observableAt<o.truth.observeUntil
      &&(!o.b.requestId||!w.b.requestId||o.b.requestId===w.b.requestId)))
      throw Error('Overlapping windows need different independent request identities');
  }
  const scoped=log.entries.filter(r=>r.channel==='popup'&&!r.test&&r.at>=capture.from&&r.at<=capture.until
    &&r.task_id&&scope.has(r.task_id)&&Number.isSafeInteger(r.notice_id));
  if(scoped.some(r=>r.stage==='queued'&&r.request_id&&!r.waiting_reason))throw Error('Delivery log does not identify the request signal');
  const rows=scoped.filter(r=>r.waiting_reason===truth.cohort.signal);
  const paints=new Map<number,number>();
  for(const row of rows)if(row.stage==='rendered'){
    const id=row.notice_id!;paints.set(id,Math.min(paints.get(id)??row.at,row.at));
  }
  const cases:Measurement[]=windows.map(w=>({id:w.truth.id,observedUntil:capture.until,alerts:[],
    restored:w.b.restored,returned:w.b.returned?{target:w.b.returned.target,verified:w.b.returned.verified}:undefined}));
  const unmatched:Measurement={id:`@unmatched-deliveries:${capture.from}:${capture.until}`,observedUntil:capture.until,alerts:[]};
  if(cases.some(c=>c.id===unmatched.id))throw Error('Reserved case ID');
  const seen=new Set<number>();
  for(const row of rows){
    if(row.stage!=='queued'||seen.has(row.notice_id!))continue;
    seen.add(row.notice_id!);
    const i=windows.findIndex(({truth:t,b})=>row.task_id===b.taskId&&row.at>=t.observableAt&&row.at<t.observeUntil
      &&(!b.requestId||row.request_id===b.requestId));
    (i<0?unmatched:cases[i]).alerts.push({renderedAt:paints.get(row.notice_id!)});
  }
  return {cases:unmatched.alerts.length?[...cases,unmatched]:cases,resourceSamples:capture.resourceSamples?.map(r=>({workingSetBytes:r.workingSetBytes,cpuMs:r.cpuMs,intervalMs:r.intervalMs}))};
}
if(import.meta.main){
  const [truthPath,logPath,capturePath]=process.argv.slice(2);
  if(!truthPath||!logPath||!capturePath)throw Error('Usage: bun scripts/request-delivery-evidence.ts truth.json notification-log-copy.json capture.json');
  const read=(path:string)=>JSON.parse(readFileSync(path,'utf8'));
  const truth:Truth=read(truthPath);
  const measurements=joinDelivery(truth,read(logPath),read(capturePath));
  // Reports contain only case IDs, timings and counts, never task IDs or titles.
  console.log(JSON.stringify({observations:measurements,report:scoreRequests(truth,measurements)},null,2));
}
