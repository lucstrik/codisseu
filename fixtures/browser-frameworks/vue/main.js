import {createApp,h,ref,version} from 'vue';
import {sendProbe,ready} from '../shared.js';
const FrameworkProbe={name:'FrameworkProbe',setup(){
  const count=ref(42);
  return ()=>h('main',[
    h('h1',{id:'marker'},'Vue actual framework'),h('p',`Vue ${version} / Unicode ação 😀`),
    h('button',{id:'counter',onClick:()=>count.value++},`Counter ${count.value}`),
    h('button',{id:'request',onClick:()=>sendProbe('vue')},'Send JSON 19 + 23'),
    h('p',{id:'network-result'},'Network pending'),
    h('small','Owned fixture. Native generic tools; no Vue props/state adapter claim.')
  ]);
}};
createApp(FrameworkProbe).mount('#app');ready('vue',version);
