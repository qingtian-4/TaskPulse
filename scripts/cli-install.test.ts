import {expect,test} from 'bun:test';
import {join} from 'node:path';
import {windowsTestProcess} from './windows-test-process';

test('PowerShell bootstrap rejects invalid releases and corrupted downloads without installing',async()=>{
  if(process.platform!=='win32')return;
  const result=await windowsTestProcess('powershell.exe',['-NoProfile','-NonInteractive','-ExecutionPolicy','Bypass','-File',join(import.meta.dir,'cli-install.test.ps1')],30000);
  expect(result.exitCode,result.stderr+'\n'+result.stdout).toBe(0);
  expect(result.stdout).toContain('PASS: release selection');
},35000);

test('installer entry point rejects an invalid version before any download or execution',async()=>{
  if(process.platform!=='win32')return;
  const result=await windowsTestProcess('powershell.exe',['-NoProfile','-NonInteractive','-ExecutionPolicy','Bypass','-File',join(import.meta.dir,'../install.ps1'),'-Version','../invalid','-DownloadOnly','-Language','en'],15000);
  expect(result.exitCode).not.toBe(0);
  expect(result.stderr).toContain('Invalid -Version');
},20000);
