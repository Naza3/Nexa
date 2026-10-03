/* TEST ONLY: resolver injection + observable connect argument, never shipped.
 * Production binary is unmodified; LD_PRELOAD explicitly supplied by test runner.
 * Accepted PUBLIC addresses are rerouted into local TLS fixture after logging.
 */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <netdb.h>
#include <arpa/inet.h>
#include <sys/socket.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <errno.h>
static void note(const char* op,const char* value){const char* p=getenv("NEXA_TEST_LOG");if(!p)return;FILE*f=fopen(p,"a");if(f){fprintf(f,"%s %s\n",op,value);fclose(f);}}
int getaddrinfo(const char* node,const char* service,const struct addrinfo* hints,struct addrinfo** res){
 static int (*real)(const char*,const char*,const struct addrinfo*,struct addrinfo**);if(!real)real=dlsym(RTLD_NEXT,"getaddrinfo");
 const char* target=node;
 if(node && strstr(node,".test")){
  if(hints && (hints->ai_flags & AI_NUMERICHOST))return EAI_NONAME;
  target="8.8.8.8";
  if(!strcmp(node,"private.test"))target="127.0.0.1";
  if(!strcmp(node,"mapped.test"))target="::ffff:127.0.0.1";
  if(!strcmp(node,"v6private.test"))target="fd00::1";
  if(!strcmp(node,"metadata.test"))target="169.254.169.254";
  if(!strcmp(node,"rebind.test")) { static int calls; target=(calls++==0)?"8.8.8.8":"127.0.0.1"; }
  note("RESOLVE",target);
 }
 return real(target,service,hints,res);
}
int connect(int fd,const struct sockaddr* a,socklen_t len){
 static int (*real)(int,const struct sockaddr*,socklen_t);if(!real)real=dlsym(RTLD_NEXT,"connect");
 char host[INET6_ADDRSTRLEN]="unknown",out[100];unsigned port=0;
 if(a->sa_family==AF_INET){const struct sockaddr_in*b=(const void*)a;inet_ntop(AF_INET,&b->sin_addr,host,sizeof(host));port=ntohs(b->sin_port);}
 else if(a->sa_family==AF_INET6){const struct sockaddr_in6*b=(const void*)a;inet_ntop(AF_INET6,&b->sin6_addr,host,sizeof(host));port=ntohs(b->sin6_port);}
 snprintf(out,sizeof(out),"%s:%u",host,port);note("CONNECT",out);
 if(!strcmp(host,"8.8.8.8") && port==443){struct sockaddr_in local={.sin_family=AF_INET};inet_pton(AF_INET,"127.0.0.1",&local.sin_addr);local.sin_port=htons(atoi(getenv("NEXA_TEST_PORT")));return real(fd,(const void*)&local,sizeof(local));}
 errno=EACCES;return -1; /* Test never reaches public network. */
}

/* Observation only: never enforce the payload cap in the test hook. */
#include <unistd.h>
#include <sys/stat.h>
static void payload_size(int fd) {
  char link[64], target[4096], value[80];
  snprintf(link, sizeof(link), "/proc/self/fd/%d", fd);
  ssize_t n = readlink(link, target, sizeof(target)-1);
  if (n <= 0) return;
  target[n] = 0;
  if (n < 8 || strcmp(target + n - 8, "/payload")) return;
  struct stat st;
  if (!fstat(fd, &st)) {
    snprintf(value, sizeof(value), "%lld", (long long)st.st_size);
    note("PAYLOAD_SIZE", value);
  }
}
ssize_t write(int fd, const void* data, size_t len) {
  static ssize_t (*real)(int, const void*, size_t);
  if (!real) real = dlsym(RTLD_NEXT, "write");
  ssize_t n = real(fd, data, len); payload_size(fd); return n;
}
int ftruncate(int fd, off_t len) {
  static int (*real)(int, off_t);
  if (!real) real = dlsym(RTLD_NEXT, "ftruncate");
  int r = real(fd, len); payload_size(fd); return r;
}
int ftruncate64(int fd, off64_t len) {
  static int (*real)(int, off64_t);
  if (!real) real = dlsym(RTLD_NEXT, "ftruncate64");
  int r = real(fd, len); payload_size(fd); return r;
}
int fallocate(int fd, int mode, off_t offset, off_t len) {
  static int (*real)(int, int, off_t, off_t);
  if (!real) real = dlsym(RTLD_NEXT, "fallocate");
  int r = real(fd, mode, offset, len); payload_size(fd); return r;
}
int fallocate64(int fd, int mode, off64_t offset, off64_t len) {
  static int (*real)(int, int, off64_t, off64_t);
  if (!real) real = dlsym(RTLD_NEXT, "fallocate64");
  int r = real(fd, mode, offset, len); payload_size(fd); return r;
}
int posix_fallocate(int fd, off_t offset, off_t len) {
  static int (*real)(int, off_t, off_t);
  if (!real) real = dlsym(RTLD_NEXT, "posix_fallocate");
  int r = real(fd, offset, len); payload_size(fd); return r;
}
int posix_fallocate64(int fd, off64_t offset, off64_t len) {
  static int (*real)(int, off64_t, off64_t);
  if (!real) real = dlsym(RTLD_NEXT, "posix_fallocate64");
  int r = real(fd, offset, len); payload_size(fd); return r;
}
