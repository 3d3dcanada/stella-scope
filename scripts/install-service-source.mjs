import fs from 'node:fs';import {createHash} from 'node:crypto';
// Fail before modifying any baseline file when a newer edit needs reconciliation.
const expected={'src/main.rs':'b12512d75c657098c9e1eb9e146a223d3b1b903c','src/config.rs':'70df8b75f84f9ba2d00ac9dfc6fcc358cc29bac4','src/moonraker.rs':'ceec557bb16333b1ce3af7c182f9d0b34561029b','src/api.rs':'5f5100483338b610a2d81ea0a3f460d44781e244','src/api/workspace.rs':'f126aefcc66e0d6d90741110676ac9862099ef99','Cargo.toml':'c0f5171dc7a7c21667fbda564f881fe3ff8d6977','Cargo.lock':'afff6a54444e7d7de9fde7c5ea3b979519e0faeb','README.md':'62259941b00038b9b1e7c5143bd305b127de8a35'};
for(const [p,sha]of Object.entries(expected)){const b=fs.readFileSync(p),got=createHash('sha1').update(`blob ${b.length}\0`).update(b).digest('hex');if(got!==sha)throw Error(`Baseline changed at ${p}: ${got}. Reconcile rather than overwrite.`);}
const staged=new Map();const edit=(p,a,b)=>{const s=staged.get(p)??fs.readFileSync(p,'utf8');if(!s.includes(a))throw Error(`Expected insertion missing in ${p}`);staged.set(p,s.replace(a,b));};
edit('src/main.rs','mod api;','mod api;\nmod telemetry;');
edit('src/config.rs','    pub objects: Vec<String>,','    pub objects: Vec<String>,\n    pub telemetry: crate::telemetry::Options,');
edit('src/config.rs','            investigation: crate::api::investigation::Limits::default(),','            investigation: crate::api::investigation::Limits::default(),\n            telemetry: crate::telemetry::Options::default(),');
edit('src/config.rs','        self.investigation.validate()?;','        self.investigation.validate()?;\n        self.telemetry.validate()?;');
edit('src/moonraker.rs','    next_discovery: f64,','    next_discovery: f64,\n    capture: Value,\n    host: Value,\n    next_host: f64,');
edit('src/moonraker.rs','missing:Vec::new(),next_discovery:0.0','missing:Vec::new(),next_discovery:0.0,capture:Value::Null,host:Value::Null,next_host:0.0');
const from=`                        self.available=self.cfg.objects.iter().filter(|spec| {
                            let name=spec.split('=').next().unwrap_or(spec);
                            let present=names.iter().any(|n|n.as_str()==Some(name));
                            if !present { self.missing.push(format!("Not reported: {name}; check its Klipper configuration name")); }
                            present
                        }).cloned().collect();`;
edit('src/moonraker.rs',from,`                        let (selected,notes,manifest)=crate::telemetry::select(&self.cfg.objects,names,&self.cfg.telemetry);
                        self.available=selected;self.missing=notes;self.capture=manifest;`);
edit('src/moonraker.rs','            self.missing.clear();','            self.missing.clear();\n            self.capture=Value::Null;');
edit('src/moonraker.rs','        Ok((Value::Object(status.clone()),warnings))',`        let mut data=status.clone();
        data.insert("_scope_capture".into(),self.capture.clone());
        if self.cfg.telemetry.host_stats {
            if ts>=self.next_host {
                self.next_host=ts+self.cfg.telemetry.host_interval_ms as f64/1000.0;
                self.host=match self.get("/machine/proc_stats") {
                    Ok(v)=>crate::telemetry::host_summary(&v,ts),
                    Err(e)=>serde_json::json!({"captured_at":ts,"error":e,"source":"GET /machine/proc_stats"}),
                };
            }
            if let Some(e)=self.host["error"].as_str(){warnings.push(format!("Host statistics unavailable: {e}"));}
            data.insert("_scope_host".into(),self.host.clone());
        }
        Ok((Value::Object(data),warnings))`);
edit('src/api/workspace.rs',' if details{',' if details{\n  v["telemetry"]=super::signals::projection(d,ts);');
edit('src/api.rs','pub mod workspace;','pub mod workspace;\npub mod signals;');
edit('src/api.rs','        "/" | "/index.html" | "/workspace"','        "/workspace"');
const routes=['','/index.html','/service',...['machine','signals','electronics','parts','diagnostics','maintenance','timeline','thermal','leveling'].map(x=>'/service/'+x)].map(x=>JSON.stringify(x||'/')).join(' | ');
const staticRoutes=`        ${routes} => Some(Response::text(200,"text/html; charset=utf-8",include_str!("../web/service.html"))),
        "/service.js" => Some(Response::text(200,"text/javascript; charset=utf-8",include_str!("../web/service.js"))),
        "/service-core.js" => Some(Response::text(200,"text/javascript; charset=utf-8",include_str!("../web/service-core.js"))),
        "/service-scene.js" => Some(Response::text(200,"text/javascript; charset=utf-8",include_str!("../web/service-scene.js"))),
        "/service.css" => Some(Response::text(200,"text/css; charset=utf-8",include_str!("../web/service.css"))),
        "/service-catalog.json" => Some(Response::text(200,"application/json; charset=utf-8",include_str!("../web/service-catalog.json"))),
`;
edit('src/api.rs','    match path {','    match path {\n'+staticRoutes);
edit('src/api.rs','    if path=="/api/workspace"','    if path=="/api/signal-history" {return signals::handle(method,target,db,cfg,status);}\n    if path=="/api/workspace"');
edit('Cargo.toml','version = "0.3.0"','version = "0.4.0"');edit('Cargo.lock','name = "stella-scope"\nversion = "0.3.0"','name = "stella-scope"\nversion = "0.4.0"');
const cfg=JSON.parse(fs.readFileSync('stella-scope.example.json'));cfg.telemetry={discover:true,max_extra_objects:96,host_stats:false,host_interval_ms:5000};staged.set('stella-scope.example.json',JSON.stringify(cfg,null,2)+'\n');
fs.copyFileSync('README.md','docs/README-0.3.md');staged.set('README.md',fs.readFileSync('docs/SERVICE-0.4.md','utf8'));for(const [p,s]of staged)fs.writeFileSync(p,s);
console.log('Applied guarded integration over exact 0.3 source. No printer or recording was modified.');
