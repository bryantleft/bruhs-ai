import {build} from "esbuild";
import {mkdir,copyFile,writeFile,cp} from "node:fs/promises";
await mkdir("dist/fonts",{recursive:true});
await build({entryPoints:["src.tsx"],outdir:"dist",bundle:true,minify:true,sourcemap:true,format:"esm",define:{"process.env.NODE_ENV":"\"production\""},loader:{".woff2":"file",".svg":"file",".png":"file"}});
await copyFile("../../../bruhs-web/fonts/MonaspaceArgonVariable.woff2","dist/fonts/MonaspaceArgonVariable.woff2");
for (const directory of ['icons','embed-icons','translations']) await cp(`node_modules/@tldraw/assets/${directory}`,`dist/tldraw-assets/${directory}`,{recursive:true});
await writeFile("dist/index.html",`<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>tldraw 5.6.0 comparator</title><link rel="stylesheet" href="/src.css"><script type="module" src="/src.js"></script></head><body style="margin:0"><div id="root" style="position:fixed;inset:0"></div></body></html>`);
