#include "NexaNetworkPolicy.h"
#include <cassert>
#include <iostream>
using namespace aria2;
int main() {
#ifdef _WIN32
  WSADATA wsa{}; assert(WSAStartup(MAKEWORD(2,2), &wsa)==0);
#endif
  const char* bad4[]={"0.0.0.0","10.1.2.3","100.64.0.1","100.127.255.255","127.1.2.3","169.254.169.254","172.16.0.1","172.31.255.255","192.0.0.9","192.0.2.1","192.88.99.2","192.168.0.1","198.18.0.1","198.19.255.255","198.51.100.1","203.0.113.1","224.0.0.1","239.1.2.3","240.0.0.1","255.255.255.255"};
  const char* good4[]={"1.1.1.1","8.8.8.8","100.63.255.255","100.128.0.0","172.15.255.255","172.32.0.0","198.17.255.255","198.20.0.0"};
  const char* bad6[]={"::","::1","::127.0.0.1","::ffff:127.0.0.1","::ffff:10.0.0.1","::ffff:169.254.169.254","fc00::1","fdff::1","fe80::1","ff02::1","64:ff9b::7f00:1","64:ff9b:1::1","100::1","100:0:0:1::1","2001::1","2001:1ff::1","2001:db8::1","2002:7f00:1::","3fff::1","3fff:fff::1","5f00::1"};
  const char* good6[]={"2606:4700:4700::1111","2001:4860:4860::8888","::ffff:8.8.8.8","3ffe::1","3fff:1000::1"};
  unsigned count=0;
  auto v4=[&](const char* s,bool expected){sockaddr_in a{};a.sin_family=AF_INET;a.sin_port=htons(443);assert(inet_pton(AF_INET,s,&a.sin_addr)==1);assert(nexa::publicDestination(reinterpret_cast<sockaddr*>(&a),sizeof(a))==expected);++count;};
  auto v6=[&](const char* s,bool expected){sockaddr_in6 a{};a.sin6_family=AF_INET6;a.sin6_port=htons(443);assert(inet_pton(AF_INET6,s,&a.sin6_addr)==1);assert(nexa::publicDestination(reinterpret_cast<sockaddr*>(&a),sizeof(a))==expected);++count;};
  for(auto s:bad4)v4(s,false);for(auto s:good4)v4(s,true);for(auto s:bad6)v6(s,false);for(auto s:good6)v6(s,true);
  sockaddr_in a{};a.sin_family=AF_INET;a.sin_port=htons(80);inet_pton(AF_INET,"8.8.8.8",&a.sin_addr);assert(!nexa::publicDestination(reinterpret_cast<sockaddr*>(&a),sizeof(a)));
  sockaddr_in6 b{};b.sin6_family=AF_INET6;b.sin6_port=htons(443);b.sin6_scope_id=1;inet_pton(AF_INET6,"2606:4700:4700::1111",&b.sin6_addr);assert(!nexa::publicDestination(reinterpret_cast<sockaddr*>(&b),sizeof(b)));
  assert(!nexa::publicDestination(nullptr,0));
  for(auto s:{"http://a/","ftp://a/","https://a/#x","https://u:p@a/","https://@a/","https://a%2e/","https://a\\b/","https://a/\r\nx"}){assert(!nexa::uriShape(s));++count;}
  for(auto s:{"https://a/","https://a:443/a%23b?x=1","https://[2606:4700::1]/"}){assert(nexa::uriShape(s));++count;}
  std::cout<<count+3<<" policy unit cases passed\n";
}
