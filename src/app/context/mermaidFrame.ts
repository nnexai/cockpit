export function mermaidRefusal(source: string): string | null {
  if (new TextEncoder().encode(source).length > 12_000 || source.split("\n").length > 200) return "Diagram exceeds the preview limit.";
  if (/%%\s*\{|^\s*---/m.test(source)) return "Diagram configuration directives are unavailable in previews.";
  return null;
}

/** Trusted library code runs only in an opaque-origin frame. Diagram text is JSON
 * data, never executable source; frame CSP also refuses remote resources. */
export function mermaidFrame(library: string, source: string, nonce: string): string {
  const literal = JSON.stringify(source).replaceAll("<", "\\u003c");
  return `<!doctype html><html><head><meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'nonce-${nonce}'; style-src 'unsafe-inline'; img-src data:; connect-src 'none'; base-uri 'none'; form-action 'none'"><style>html,body{margin:0;background:#10151d;color:#d7dde7;font:14px sans-serif}#diagram{padding:12px}svg{max-width:100%;height:auto}</style></head><body><div id="diagram"></div><script nonce="${nonce}">${library.replace(/<\/script/gi, "<\\/script")}</script><script nonce="${nonce}">
(async()=>{try{mermaid.initialize({startOnLoad:false,securityLevel:'strict',theme:'dark',maxTextSize:12000,maxEdges:160,suppressErrorRendering:true,flowchart:{htmlLabels:false},secure:['securityLevel','maxTextSize','maxEdges','startOnLoad','suppressErrorRendering','flowchart']});const result=await mermaid.render('diagram-svg',${literal});document.getElementById('diagram').innerHTML=result.svg;parent.postMessage({kind:'cockpit-mermaid',nonce:'${nonce}',ok:true,height:Math.min(640,Math.max(80,document.body.scrollHeight))},'*')}catch(_){parent.postMessage({kind:'cockpit-mermaid',nonce:'${nonce}',ok:false},'*')}})();
</script></body></html>`;
}
