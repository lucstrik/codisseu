# Native browser inspector fixture

Original Codisseu test content, no external assets/accounts or valid credentials.
Token/cookie values are deliberately inert literals for mask/reveal/copy checks.

The owned stdlib server `server.py` serves these files on an ephemeral IPv4
loopback port and records real requests. The UI tests own/reap that process.
`POST /api/echo`
receives JSON19+23 and an Authorization header, returns JSONanswer42/status201
after a bounded server delay, and sets its own response header. The document
response seeds a HttpOnly/SameSiteStrict cookie and a readable SameSiteLax cookie.

The page seeds separate localStorage/sessionStorage entries, exposes DOM/text/
attribute controls and an open shadow root. `computeProbe` pauses on a real
debugger statement with localsleft19/right23/result42; Step advances through
doubled84 and Resume changes the page title. Native inspector controls must
operate on this actual page and must preserve the dirty editor document.

`jwt_token` has readable fixture JSON (subfixture-user, answer42) and a deliberately
invalid inert signature. Decoding it never verifies authenticity or logs in.
