import { defineConfig } from 'vite';
export default defineConfig({
  server: {host:'127.0.0.1',strictPort:true},
  plugins: [{
    name:'codisseu-owned-json-fixture',
    configureServer(server) {
      server.middlewares.use('/api/echo', (req,res) => {
        if(req.method!=='POST'){res.statusCode=405;res.end();return;}
        const chunks=[];let size=0;
        req.on('data',part=>{size+=part.length;if(size>65536)req.destroy();else chunks.push(part)});
        req.on('end',()=>{
          try {
            const value=JSON.parse(Buffer.concat(chunks).toString('utf8'));
            if(!Number.isInteger(value.left)||!Number.isInteger(value.right))throw Error('invalid operands');
            res.statusCode=201;res.setHeader('Content-Type','application/json; charset=utf-8');
            res.setHeader('Cache-Control','no-store');res.setHeader('X-Fixture','framework-native');
            res.end(JSON.stringify({answer:value.left+value.right,framework:value.framework,message:value.message}));
          }catch {res.statusCode=400;res.end('{}');}
        });
      });
    }
  }]
});
