const { chromium } = require('/home/linuxbrew/.linuxbrew/lib/node_modules/@playwright/cli/node_modules/playwright-core');
const fs = require('fs');
(async()=>{
 const browser = await chromium.connectOverCDP('http://127.0.0.1:9229');
 const ctx = browser.contexts()[0];
 let page = ctx.pages().find(p=>p.url().startsWith('http://127.0.0.1:8765'));
 if(!page) throw new Error('fixture page missing');
 const out='planning/product-atlas-2026-09-12/runtime/browser-extension';
 const record={scenario:'browser.runtime-extension-smoke',session:'atlas-0912-browser',tab:{url:page.url(),title:await page.title()},viewport:{initial:page.viewportSize()},events:[]};
 await page.setViewportSize({width:1440,height:900});
 await page.screenshot({path:out+'/screenshots/fixture-1440x900.png',fullPage:true});
 record.events.push({action:'page screenshot',viewport:'1440x900',result:'PASS'});
 const state=await page.evaluate(()=>({hasHost:!!document.querySelector('cockpit-feedback'),host:document.querySelector('cockpit-feedback')?.shadowRoot?true:false,controls:document.querySelector('cockpit-feedback')?.shadowRoot?.querySelector('.controls')?.getAttribute('aria-label')||null}));
 record.extensionInjection=state;
 if(state.hasHost){
   await page.evaluate(()=>{const h=document.querySelector('cockpit-feedback'); const r=h.shadowRoot; r.querySelector('.mode').value='annotate'; r.querySelector('.mode').dispatchEvent(new Event('change',{bubbles:true}));});
   await page.mouse.move(380,280); await page.mouse.down(); await page.mouse.move(520,330); await page.mouse.up();
   await page.waitForTimeout(150);
   const ann=await page.evaluate(()=>{const r=document.querySelector('cockpit-feedback').shadowRoot; return {mode:r.querySelector('.mode').value,marks:r.querySelectorAll('.marks *').length,hint:r.querySelector('.hint')?.textContent||''};});
   record.events.push({action:'annotate freehand',result:ann});
   await page.setViewportSize({width:1024,height:640});
   await page.screenshot({path:out+'/screenshots/fixture-1024x640-annotated.png',fullPage:true});
   record.events.push({action:'annotated screenshot',viewport:'1024x640',result:'PASS'});
   await page.reload(); await page.waitForTimeout(250);
   record.events.push({action:'reload stale draft probe',result:await page.evaluate(()=>({host:!!document.querySelector('cockpit-feedback'),url:location.href}))});
 }
 const extId='fignfifoniblkonapihmkfakmlgkbkcf';
 try {
   const popup=await ctx.newPage(); await popup.goto(`chrome-extension://${extId}/popup.html`,{waitUntil:'domcontentloaded',timeout:5000}); await popup.waitForTimeout(300);
   record.popup={url:popup.url(),title:await popup.title(),status:await popup.locator('#status').textContent(),connection:await popup.locator('#connection').textContent().catch(()=>null),pending:await popup.locator('#pending').textContent().catch(()=>null)};
   await popup.screenshot({path:out+'/screenshots/popup-disconnected.png'});
   record.events.push({action:'popup disconnected',result:record.popup});
 } catch (error) {
   record.popup={result:'INCONCLUSIVE',error:String(error).split('\\n')[0]};
   record.events.push({action:'popup disconnected',result:'INCONCLUSIVE',reason:record.popup.error});
 }
 await fs.promises.writeFile(out+'/receipts/runtime-smoke.json',JSON.stringify(record,null,2));
 await browser.close();
})().catch(e=>{console.error(e.stack);process.exitCode=1});
