import {test,expect} from 'bun:test';
import {scoreRequests,type Truth,type Measurements} from '../scripts/request-evaluation';
const truth:Truth={cohort:{agent:'fixture',version:'fixture-v1',platform:'windows',signal:'approval',lane:'fixed'},oracle:'authored_replay',cases:[
  {id:'noise',expected:'no_request',observableAt:1000,observeUntil:4000,path:'event'},
  {id:'request',expected:'request',observableAt:5000,observeUntil:9000,path:'poll',pollIntervalMs:1000,restart:true,returnTarget:'details'},
  {id:'unavailable',expected:'unsupported',observableAt:9000,observeUntil:12000,path:'event'},
]};
const observations:Measurements={cases:[
  {id:'noise',observedUntil:4000,alerts:[]},
  {id:'request',observedUntil:9000,alerts:[{renderedAt:6400}],restored:true,returned:{target:'details',verified:true}},
]};
test('scores fixed observations against authored truth without crediting unsupported input',()=>{
  const result=scoreRequests(truth,observations);
  expect(result).toMatchObject({passed:true,positives:1,negatives:1,unsupported:1,missed:0,falseAlerts:0,
    latencyByPath:[{path:'poll',limitMs:4000,samples:1,p95Ms:1400}],
    recovery:{expected:1,recovered:1},returns:{details:{expected:1,verified:1},session:{expected:0,verified:0}}});
  expect(scoreRequests({...truth,cases:[truth.cases[2]]},{cases:[]})).toMatchObject({passed:false,falseNegativeRate:null,unsupported:1});
});
test('duplicates, unrelated alerts, unpainted delivery and incomplete recovery fail the fixed gate',()=>{
  expect(scoreRequests(truth,{cases:[observations.cases[0],{...observations.cases[1],alerts:[{renderedAt:6000},{renderedAt:6500}]},
    {id:'other',alerts:[{renderedAt:8000}],observedUntil:9000}]})).toMatchObject({passed:false,falseAlerts:2,unmatchedAlerts:1});
  expect(scoreRequests(truth,{cases:[{...observations.cases[1],alerts:[{}],restored:false,observedUntil:6000}]}))
    .toMatchObject({passed:false,missed:1,unrendered:1,incomplete:2,recovery:{expected:1,recovered:0}});
});
test('real thresholds require separate host truth and enough positive and negative samples',()=>{
  const real:Truth={...truth,cohort:{...truth.cohort,lane:'real'},oracle:'host_observation'};
  expect(scoreRequests(real,observations)).toMatchObject({passed:false,sampleReady:false});
  expect(()=>scoreRequests({...real,oracle:'authored_replay'},observations)).toThrow();
  const cases=Array.from({length:100},(_,i)=>truth.cases.slice(0,2).map(r=>({...r,id:r.id+i}))).flat();
  const actual=Array.from({length:100},(_,i)=>observations.cases.map(r=>({...r,id:r.id+i}))).flat();
  expect(scoreRequests({...real,cases},{cases:actual}).passed).toBe(true);
});

test('real log join separates actual paints, unmatched notifications and manual reopen',async()=>{
  const {joinDelivery}=await import('../scripts/request-delivery-evidence');
  const real:Truth={...truth,cohort:{...truth.cohort,lane:'real'},oracle:'host_observation'};
  const capture={from:900,until:12500,complete:true,taskIds:['native-fixture'],bindings:truth.cases.map(t=>({id:t.id,taskId:'native-fixture',requestId:t.expected==='request'?'native-request':undefined}))};
  const row=(stage:string,id:number,at:number)=>({stage,at,channel:'popup',notice_id:id,task_id:'native-fixture',request_id:'native-request',waiting_reason:'approval',test:false});
  const actual=joinDelivery(real,{entries:[row('queued',1,5800),row('rendered',1,5900),row('reopened',2,6200),row('rendered',2,6500),row('queued',3,4500),{...row('queued',4,5600),request_id:null,waiting_reason:'turn_ended'}]},capture);
  expect(actual.cases.find(c=>c.id==='request')?.alerts).toEqual([{renderedAt:5900}]);
  expect(actual.cases.at(-1)?.alerts).toHaveLength(1);
  expect(()=>joinDelivery(real,{entries:[]},{...capture,complete:false})).toThrow('uninterrupted');
  expect(()=>joinDelivery(real,{entries:Array.from({length:500},()=>row('queued',1,5800))},capture)).toThrow('untruncated');
});
