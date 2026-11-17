import React,{useState,version} from 'react';
import {createRoot} from 'react-dom/client';
import {sendProbe,ready} from '../shared.js';
function FrameworkProbe(){
  const [count,setCount]=useState(42);
  return React.createElement('main',null,
    React.createElement('h1',{id:'marker'},'React actual framework'),
    React.createElement('p',null,`React ${version} / Unicode ação 😀`),
    React.createElement('button',{id:'counter',onClick:()=>setCount(n=>n+1)},`Counter ${count}`),
    React.createElement('button',{id:'request',onClick:()=>sendProbe('react')},'Send JSON 19 + 23'),
    React.createElement('p',{id:'network-result'},'Network pending'),
    React.createElement('small',null,'Owned fixture. Native generic tools; no React props/state adapter claim.'));
}
createRoot(document.querySelector('#app')).render(React.createElement(FrameworkProbe));
ready('react',version);
