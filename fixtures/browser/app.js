"use strict";
function compute(left, right) {
    const result = left + right;
    window.lastResult = result;
    document.getElementById("result").textContent = String(result);
    return result;
}
document.getElementById("calculate").addEventListener("click", () => compute(19, 23));
fetch("/data").then(response => response.json()).then(data => {
    document.getElementById("fetch-result").textContent = `Fetch: ${data.value}`;
});
