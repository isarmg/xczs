const {test,expect} = require("./fixtures");
const {randomUUID} = require("node:crypto");

const tag = (id,name) => ({id,name,color:null,file_count:0});
const tagsPage = (tags,next_cursor=null) => ({tags,previous_cursor:null,next_cursor});
const reply = (route,payload) => route.fulfill({status:200,contentType:"application/json",body:JSON.stringify(payload)});

async function openPicker(page) {
  await page.locator(".search-tags summary").click();
  await expect(page.locator(".search-tags")).toHaveAttribute("open", "");
}

// These exercise the mounted Rust components and their real serialized fetch
// queue. They replace the old React-hook scheduling fixtures with DOM and
// network assertions at the same lifecycle boundaries.
test("Tag drafts retain selected off-page IDs and expose deleted IDs after mutations",async({appPage:page})=>{
  await expect(page.locator("html")).toHaveAttribute("data-xcss-runtime","xcss-web");
  let changed=false;
  await page.route("**/api/v1/file-tags/tags*",route=>{
    if(route.request().method()==="POST") {
      changed=true;
      return reply(route,{id:3});
    }
    const cursor=new URL(route.request().url()).searchParams.get("cursor");
    return reply(route,changed?tagsPage([tag(2,"Remaining")]):cursor?tagsPage([tag(2,"Second page")]):tagsPage([tag(1,"Selected before mutation")],"page-two"));
  });
  await openPicker(page);
  const all=page.getByRole("listbox",{name:"All tags",exact:true});
  await all.selectOption("1");
  await page.locator(".search-tag-pager").getByRole("button",{name:"Next page",exact:true}).first().click();
  await expect(all.locator("option")).toHaveText(["Selected before mutation","Second page"]);
  expect(await all.inputValue()).toBe("1");
  expect(new URL(page.url()).searchParams.has("all")).toBe(false);
  await page.locator('.xcss-header-navigation a[href="#tags"]').click();
  await page.getByRole("textbox",{name:"Tag name",exact:true}).fill("Mutation refresh");
  await page.getByRole("button",{name:"Create tag",exact:true}).click();
  await expect(page.getByRole("table",{name:"Tag list",exact:true})).toContainText("Remaining");
  await page.locator('.xcss-header-navigation a[href="#files"]').click();
  await expect(all.locator("option")).toHaveText(["#1","Remaining"]);
  expect(await all.inputValue()).toBe("1");
  await expect(page.locator(".search-tag-pager").getByRole("button",{name:"Next page",exact:true}).first()).toBeDisabled();
  await all.selectOption([]);
  await page.locator(".search-tag-actions").getByRole("button",{name:"Search",exact:true}).click();
  expect(new URL(page.url()).searchParams.has("all")).toBe(false);
  await expect(page.locator(".search-tags")).not.toHaveAttribute("open", "");
});

test("Closing a tag picker aborts its request and a late response cannot restore old options",async({appPage:page})=>{
  let releaseOld;
  const oldGate=new Promise(resolve=>{releaseOld=resolve;});
  let reads=0;
  await page.route("**/api/v1/file-tags/tags*",async route=>{
    reads++;
    if(reads===1) {
      await oldGate;
      try { await reply(route,tagsPage([tag(1,"Obsolete")],"obsolete-cursor")); } catch { /* Expected aborted fetch. */ }
    } else await reply(route,tagsPage([tag(2,"Current")]));
  });
  await openPicker(page);
  await expect.poll(()=>reads).toBe(1);
  await page.locator(".search-tags summary").click();
  await openPicker(page);
  const all=page.getByRole("listbox",{name:"All tags",exact:true});
  await expect(all.locator("option")).toHaveText(["Current"]);
  releaseOld();
  await expect(all.locator("option")).toHaveText(["Current"]);
  await expect(page.locator(".search-tag-pager").getByRole("button",{name:"Next page",exact:true}).first()).toBeDisabled();
  expect(reads).toBe(2);
});

test("Aborting a tag Retry-After delay promptly releases the shared request queue",async({appPage:page})=>{
  let reads=0;
  await page.route("**/api/v1/file-tags/tags*",route=>{
    reads++;
    if(reads===1) return route.fulfill({status:429,contentType:"application/json",headers:{"Retry-After":"2"},body:JSON.stringify({code:"too_many_requests",message:"Try later",retryable:true})});
    return reply(route,tagsPage([tag(2,"Queue released")]));
  });
  await openPicker(page);
  await expect.poll(()=>reads).toBe(1);
  // Wait for the response body to enter Retry-After, without advancing its timer.
  await page.waitForTimeout(50);
  await page.locator(".search-tags summary").click();
  const started=Date.now();
  await openPicker(page);
  await expect(page.getByRole("listbox",{name:"All tags",exact:true}).locator("option")).toHaveText(["Queue released"],{timeout:1000});
  expect(Date.now()-started).toBeLessThan(1500);
  expect(reads).toBe(2);
});

test("A tag read deadline displays an error and Retry rather than leaving an obsolete loading state",async({appPage:page})=>{
  await page.clock.install();
  let reads=0,releaseOld;
  const gate=new Promise(resolve=>{releaseOld=resolve;});
  await page.route("**/api/v1/file-tags/file?*",async route=>{
    reads++;
    if(reads===1) {
      await gate;
      try { await reply(route,{file_id:null,tags:[],tags_has_more:false}); } catch { /* Timed out. */ }
    } else await reply(route,{file_id:null,tags:[],tags_has_more:false});
  });
  await page.locator('[data-file-action="tags"]').click();
  await page.getByRole("link",{name:"download-me.txt",exact:true}).click();
  await expect.poll(()=>reads).toBe(1);
  const editor=page.getByRole("region",{name:"File tags",exact:true});
  await expect(editor).toContainText("Loading");
  await page.clock.fastForward(5001);
  await expect(editor.getByRole("alert")).toContainText("Could not read or update tags");
  const retry=editor.getByRole("button",{name:"Retry",exact:true});
  await expect(retry).toBeEnabled();
  await retry.click();
  await expect(editor.getByRole("button",{name:"Scan entire library",exact:true})).toBeVisible();
  releaseOld();
  expect(reads).toBe(2);
});

test("Completing tag creation preserves a newer name draft typed while the request was pending",async({appPage:page})=>{
  const committed=`Committed tag ${randomUUID().slice(0,8)}`;
  let releaseCreate,markStarted;
  const gate=new Promise(resolve=>{releaseCreate=resolve;});
  const started=new Promise(resolve=>{markStarted=resolve;});
  await page.route("**/api/v1/file-tags/tags",async route=>{
    if(route.request().method()!=="POST") return route.fallback();
    markStarted();
    await gate;
    await route.continue();
  });
  await page.locator('.xcss-header-navigation a[href="#tags"]').click();
  const name=page.getByRole("textbox",{name:"Tag name",exact:true});
  await name.fill(committed);
  await page.getByRole("button",{name:"Create tag",exact:true}).click();
  await started;
  await name.fill("Newer unsubmitted draft");
  releaseCreate();
  await expect(page.getByRole("table",{name:"Tag list",exact:true})).toContainText(committed);
  await expect(name).toHaveValue("Newer unsubmitted draft");
});
