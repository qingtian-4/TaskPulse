/** Operate the task filter popover through its buttons in native UI tests. */
export async function clickDashboardFilter(evaluate:(expression:string)=>Promise<unknown>,selector:string) {
  await evaluate(`if (!document.querySelector('.filters')) document.querySelector('.filter-toggle').click()`);
  await evaluate(`(() => {
    const button=document.querySelector(${JSON.stringify(selector)});
    if (!button) throw Error('Dashboard filter action is missing');
    if (button.disabled) document.querySelector('.filter-toggle').click();
    else button.click();
  })()`);
}
