// Card bottoms include project headings when grouping by project. Never size from the viewport's scrollHeight.
export function dashboardHeightForRows(chrome:number,bottoms:number[],expanded=false,empty=96):number {
  if(expanded)return 840;
  const body=bottoms.length?bottoms[Math.min(4,bottoms.length)-1]:empty;
  return Math.ceil(Math.max(280,Math.min(1600,chrome+body+8)));
}

export function measureDashboard(panel:HTMLElement,expanded=false):number {
  const main=panel.querySelector<HTMLElement>('.task-scroll');
  if(!main)return 400;
  const top=main.getBoundingClientRect().top;
  const rows=[...main.querySelectorAll<HTMLElement>('.task-item')].slice(0,4).map(row=>row.getBoundingClientRect().bottom-top+main.scrollTop);
  const chrome=[...panel.children].reduce((sum,node)=>{
    const style=getComputedStyle(node);
    if(node===main||style.position==='absolute'||style.position==='fixed')return sum;
    return sum+node.getBoundingClientRect().height+(parseFloat(style.marginTop)||0)+(parseFloat(style.marginBottom)||0);
  },2);
  return dashboardHeightForRows(chrome,rows,expanded,main.querySelector('.empty')?.getBoundingClientRect().height??96);
}
