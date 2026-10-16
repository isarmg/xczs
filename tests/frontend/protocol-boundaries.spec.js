const {test,expect}=require("./fixtures");

test("A Fetch 401 settles at headers without consuming a stalled body or querying a mutation job",async({appPage:page})=>{
  let jobs=0;
  const messages=[];
  page.on("console",message=>messages.push(message.text()));
  page.on("request",request=>{if(new URL(request.url()).pathname.startsWith("/__xczs__/api/jobs/")) jobs++;});
  await page.evaluate(()=>{
    const fetch=window.fetch.bind(window);
    window.fetch=(input,options)=>{
      if(options?.method==="DELETE"&&String(input).endsWith("/delete-me.txt")) {
        const body=new ReadableStream({start(){}});
        const getReader=body.getReader.bind(body);
        body.getReader=(...args)=>{console.info("xczs-test-401-body-read");return getReader(...args);};
        return Promise.resolve(new Response(body,{status:401,headers:{"content-type":"application/json"}}));
      }
      return fetch(input,options);
    };
  });
  await page.locator('[data-file-action="delete"]').click();
  await page.getByRole("link",{name:"delete-me.txt",exact:true}).click();
  const navigated=page.waitForEvent("framenavigated",{predicate:frame=>frame===page.mainFrame(),timeout:5000});
  await page.getByRole("dialog",{name:"Delete item",exact:true}).getByRole("button",{name:"Delete",exact:true}).click();
  await navigated;
  expect(messages).not.toContain("xczs-test-401-body-read");
  expect(jobs).toBe(0);
  await expect(page.getByRole("link",{name:"delete-me.txt",exact:true})).toBeVisible();
});
