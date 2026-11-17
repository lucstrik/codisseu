import './style.css';
export async function sendProbe(framework) {
  const response=await fetch('/api/echo', {method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({left:19,right:23,framework,message:'ação😀'})});
  const value=await response.json();
  if(response.status!==201||value.answer!==42)throw Error('unexpected fixture response');
  window.networkAnswer=value.answer;
  document.title=`${framework} network42`;
  document.querySelector('#network-result').textContent=`POST ${response.status} / answer ${value.answer}`;
}
export function ready(framework,version) {
  localStorage.setItem(`${framework}-fixture`,'native-storage42');
  window.fixtureFramework=framework;
  window.fixtureVersion=version;
  document.title=`${framework} ready42`;
}
