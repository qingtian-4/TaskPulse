/** Score observer evidence against an independent oracle. No Agent data is read
 * or written here. Supply one cohort per Agent, version, platform and signal. */
import { readFileSync } from 'node:fs';

export type Path = 'event' | 'poll';
export type Target = 'session' | 'host_window' | 'details';
export interface TruthCase {
  id: string;
  expected: 'request' | 'no_request' | 'unsupported';
  observableAt: number;
  observeUntil: number;
  path: Path;
  pollIntervalMs?: number;
  restart?: boolean;
  returnTarget?: Target;
}
export interface Truth {
  cohort: { agent: string; version: string; platform: string; signal: 'approval' | 'question'; lane: 'fixed' | 'real' };
  oracle: 'authored_replay' | 'host_observation';
  cases: TruthCase[];
}
export interface Measurement {
  id: string;
  observedUntil: number;
  alerts: { renderedAt?: number }[];
  restored?: boolean;
  returned?: { target: Target; verified: boolean };
}
export interface Measurements {
  cases: Measurement[];
  resourceSamples?: { workingSetBytes?: number; cpuMs?: number; intervalMs: number }[];
}

const p95 = (values:number[]) => values.length ? [...values].sort((a,b)=>a-b)[Math.ceil(values.length*.95)-1] : null;
function require(condition:unknown,message:string): asserts condition {
  if(!condition)throw Error(message);
}
const validTime=(n:unknown)=>typeof n==='number'&&Number.isSafeInteger(n)&&n>=0;
const target=(t:unknown)=>t==='session'||t==='host_window'||t==='details';

export function scoreRequests(truth:Truth, measurements:Measurements) {
  const c=truth.cohort;
  require(c&&[c.agent,c.version,c.platform].every(x=>typeof x==='string'&&x.trim()),'Agent, version and platform are required');
  require(c.signal==='approval'||c.signal==='question','Unknown signal');
  require(c.lane==='fixed'&&truth.oracle==='authored_replay'||c.lane==='real'&&truth.oracle==='host_observation','Use an independent oracle for the selected lane');
  require(Array.isArray(truth.cases)&&truth.cases.length>0&&Array.isArray(measurements.cases),'Missing evaluation cases');
  const expected=new Map<string,TruthCase>();
  const samples=new Map<string,Measurement>();
  for(const row of truth.cases){
    require(typeof row.id==='string'&&row.id.length>0&&!expected.has(row.id),'Duplicate or missing truth ID');
    require(['request','no_request','unsupported'].includes(row.expected),'Unknown truth outcome');
    require(validTime(row.observableAt)&&validTime(row.observeUntil)&&row.observeUntil>row.observableAt,'Invalid observation window');
    require(row.path==='event'||row.path==='poll','Unknown input path');
    if(row.path==='poll')require(validTime(row.pollIntervalMs)&&row.pollIntervalMs!>0,'Poll interval is required');
    require(row.returnTarget===undefined||target(row.returnTarget),'Unknown return target');
    expected.set(row.id,row);
  }
  for(const sample of measurements.cases){
    require(typeof sample.id==='string'&&sample.id.length>0&&!samples.has(sample.id),'Duplicate or missing measurement ID');
    require(validTime(sample.observedUntil)&&Array.isArray(sample.alerts),'Invalid measurement window');
    for(const alert of sample.alerts)require(alert.renderedAt===undefined||validTime(alert.renderedAt),'Invalid render timestamp');
    require(!sample.returned||target(sample.returned.target)&&typeof sample.returned.verified==='boolean','Invalid return result');
    samples.set(sample.id,sample);
  }
  for(const r of measurements.resourceSamples??[]){
    require(validTime(r.intervalMs)&&r.intervalMs>0,'Invalid resource sample duration');
    require(r.workingSetBytes===undefined||validTime(r.workingSetBytes),'Invalid working set');
    require(r.cpuMs===undefined||validTime(r.cpuMs),'Invalid CPU time');
  }

  let positives=0,negatives=0,unsupported=0,missed=0,unqueued=0,falseAlerts=0,unmatchedAlerts=0,incomplete=0,unrendered=0;
  let recoveryExpected=0,recovered=0;
  const returns:Record<Target,{expected:number;verified:number}>={session:{expected:0,verified:0},host_window:{expected:0,verified:0},details:{expected:0,verified:0}};
  const latency=new Map<string,{path:Path;limitMs:number;times:number[]}>();
  for(const row of expected.values()){
    const sample=samples.get(row.id);
    if(row.expected==='unsupported'){
      unsupported++;
      if(sample?.alerts.length)falseAlerts+=sample.alerts.length;
      continue;
    }
    const validWindow=sample&&sample.observedUntil>=row.observeUntil;
    if(!validWindow)incomplete++;
    if(row.expected==='no_request'){
      negatives++;
      falseAlerts+=sample?.alerts.length??0;
      continue;
    }
    positives++;
    const rendered=sample?.alerts.find(a=>a.renderedAt!==undefined&&a.renderedAt>=row.observableAt&&a.renderedAt<row.observeUntil);
    const deliveries=sample?.alerts.length??0;
    falseAlerts+=Math.max(0,deliveries-1);
    if(!deliveries)unqueued++;
    if(!rendered){
      missed++;
      if(sample?.alerts.some(a=>a.renderedAt===undefined))unrendered++;
    } else {
      const limitMs=row.path==='event'?2000:2*row.pollIntervalMs!+2000;
      const key=row.path+':'+limitMs;
      const bucket=latency.get(key)??{path:row.path,limitMs,times:[]};
      bucket.times.push(rendered.renderedAt!-row.observableAt);
      latency.set(key,bucket);
    }
    if(row.restart){recoveryExpected++;if(sample?.restored===true)recovered++;}
    if(row.returnTarget){
      const result=returns[row.returnTarget];result.expected++;
      if(sample?.returned?.target===row.returnTarget&&sample.returned.verified)result.verified++;
    }
  }
  for(const sample of samples.values())if(!expected.has(sample.id)){unmatchedAlerts+=sample.alerts.length;}
  falseAlerts+=unmatchedAlerts;
  const latencyByPath=[...latency.values()].map(({path,limitMs,times})=>({path,limitMs,samples:times.length,p95Ms:p95(times)!}));
  const falsePositiveRate=negatives?falseAlerts/negatives:null;
  const falseNegativeRate=positives?missed/positives:null;
  const sampleReady=c.lane==='fixed'?positives>0&&negatives>0:positives>=100&&negatives>=100;
  const passed=sampleReady&&incomplete===0&&
    (c.lane==='fixed'?falseAlerts===0&&missed===0:falsePositiveRate!<=.01&&falseNegativeRate!<=.01)&&
    recovered===recoveryExpected&&Object.values(returns).every(r=>r.expected===r.verified)&&
    latencyByPath.every(r=>r.p95Ms<=r.limitMs);
  return {cohort:{agent:c.agent,version:c.version,platform:c.platform,signal:c.signal,lane:c.lane},passed,sampleReady,positives,negatives,unsupported,missed,unqueued,falseAlerts,unmatchedAlerts,incomplete,unrendered,
    falsePositiveRate,falseNegativeRate,latencyByPath,recovery:{expected:recoveryExpected,recovered},returns,
    resources:{samples:measurements.resourceSamples?.length??0,workingSetP95Bytes:p95((measurements.resourceSamples??[]).flatMap(r=>r.workingSetBytes??[])),
      cpuP95Percent:p95((measurements.resourceSamples??[]).flatMap(r=>r.cpuMs===undefined?[]:100*r.cpuMs/r.intervalMs))}};
}

if(import.meta.main){
  const [truthFile,...files]=process.argv.slice(2);
  if(!truthFile||!files.length)throw Error('Usage: bun scripts/request-evaluation.ts <independent-truth.json> <observations.json> [more-disjoint-captures.json ...]');
  const inputs:Measurements[]=files.map(path=>{
    const json=JSON.parse(readFileSync(path,'utf8'));
    return json.observations??json;
  });
  const report=scoreRequests(JSON.parse(readFileSync(truthFile,'utf8')),{
    cases:inputs.flatMap(x=>x.cases),resourceSamples:inputs.flatMap(x=>x.resourceSamples??[]),
  });
  console.log(JSON.stringify(report,null,2));
  process.exitCode=report.passed?0:1;
}
