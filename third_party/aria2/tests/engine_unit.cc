#include "Request.h"
#include "SocketCore.h"
#include "Exception.h"
#include <cassert>
#include <iostream>
int main() {
#ifdef _WIN32
 WSADATA wsa{}; assert(WSAStartup(MAKEWORD(2,2), &wsa)==0);
#endif
 // Invoke actual SocketCore (including getaddrinfo), not just classifier.
 // The same socket instance is reused to exercise the gate repeatedly.
 aria2::SocketCore socket;
 for (auto host : {"127.0.0.1", "10.0.0.1", "169.254.169.254", "::1"}) {
  bool blocked=false;
  try { socket.establishConnection(host,443); }
  catch (const aria2::Exception& e) {
   blocked=std::string(e.what()).find("Nexa policy: destination rejected")!=std::string::npos;
  }
  assert(blocked);
 }
 std::cout<<"4 actual SocketCore private destination cases passed\n";
 unsigned n=0;
 for(auto s:{"https://example.test/","https://example.test:443/a","https://[2606:4700::1]/"}){aria2::Request r;assert(r.setUri(s));++n;}
 for(auto s:{"http://a/","https://a:80/","https://a:444/","ftp://a/","https://a/#frag","https://@a/","https://u:p@a/","https://a%2e/","https://[fe80::1%25eth0]/"}){aria2::Request r;assert(!r.setUri(s));++n;}
 for(auto s:{"https://cdn.test/a","//cdn.test/a","/b","../b","?x=1","https://cdn.test/a%23b"}){aria2::Request r;assert(r.setUri("https://public.test/a"));assert(r.redirectUri(s));assert(r.getProtocol()=="https"&&r.getPort()==443);++n;}
 for(auto s:{"http://a/","ftp://a/","https://a:444/","//a:80/","https://@a/","https://u:p@a/","#frag","/a#frag"}){aria2::Request r;assert(r.setUri("https://public.test/a"));assert(!r.redirectUri(s));++n;}
 std::cout<<n<<" Request parser/redirect cases passed\n";
}
