import '@angular/compiler';
import {Component,VERSION,provideZonelessChangeDetection} from '@angular/core';
import {bootstrapApplication} from '@angular/platform-browser';
import {sendProbe,ready} from '../shared.js';
class FrameworkProbe {
  count=42;
  version=VERSION.full;
  increment(){this.count++;}
  request(){sendProbe('angular');}
}
Component({selector:'app-framework',standalone:true,template:`
  <main><h1 id="marker">Angular actual framework</h1>
  <p>Angular {{version}} / Unicode ação 😀</p>
  <button id="counter" (click)="increment()">Counter {{count}}</button>
  <button id="request" (click)="request()">Send JSON 19 + 23</button>
  <p id="network-result">Network pending</p>
  <small>Owned fixture. Native generic tools; no Angular component adapter claim.</small></main>`})(FrameworkProbe);
document.querySelector('#app').innerHTML='<app-framework></app-framework>';
bootstrapApplication(FrameworkProbe,{providers:[provideZonelessChangeDetection()]}).then(()=>ready('angular',VERSION.full));
