HTTP/1.1 202 Accepted
Host: localhost
Date: %DATE%
Connection: close
X-Powered-By: PHP/8.4.27
X-Many: a
X-Many: b
X-First: 2
Content-type: text/plain;charset=UTF-8
Set-Cookie: plain=a%20b%26c
Set-Cookie: opts=v; expires=Wed, 18 May 2033 03:33:20 GMT; Max-Age=N; path=/p; secure; HttpOnly; SameSite=Lax
Set-Cookie: raw=a+b
Set-Cookie: del=deleted; expires=Thu, 01 Jan 1970 00:00:01 GMT; Max-Age=N

bool(false)
["X-Powered-By: PHP\/8.4.27","X-Many: a","X-Many: b","X-First: 2","Content-type: text\/plain;charset=UTF-8","Set-Cookie: plain=a%20b%26c","Set-Cookie: opts=v; expires=Wed, 18 May 2033 03:33:20 GMT; Max-Age=NN; path=\/p; secure; HttpOnly; SameSite=Lax","Set-Cookie: raw=a+b","Set-Cookie: del=deleted; expires=Thu, 01 Jan 1970 00:00:01 GMT; Max-Age=NN"]
bool(true)
int(202)
