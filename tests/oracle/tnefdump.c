/*
 * Attribute-level dump of a TNEF stream through the C libytnef, used as the
 * differential oracle for ytnef-core. The Rust side (crates/ytnef-cli/tests/
 * differential.rs) renders its parse result in exactly this format.
 *
 * Output: "rc=<n>" and, on success, one "key=value" line per field. Byte
 * strings print as "<len>:<escaped>" (printable ASCII kept, rest as \xHH);
 * blobs print as "<len>:<fnv1a-64 hex>". "rtf" is the message's
 * PR_RTF_COMPRESSED run through DecompressRTF: a blob, or "error".
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <fcntl.h>
#include <unistd.h>
#include <ytnef.h>

static void esc(const char *k, const BYTE *p, size_t n) {
  size_t i;
  printf("%s=%zu:", k, n);
  for (i = 0; i < n; i++) {
    if (p[i] >= 0x20 && p[i] < 0x7f && p[i] != '\\')
      putchar(p[i]);
    else
      printf("\\x%02x", p[i]);
  }
  putchar('\n');
}

static void vl(const char *k, variableLength *v) {
  if (v->size > 0 && v->data) esc(k, v->data, v->size);
}

/* fixed char[] fields: what a C caller sees with strnlen */
static void fixed(const char *k, const char *s, size_t cap) {
  size_t n = strnlen(s, cap);
  if (n > 0) esc(k, (const BYTE *)s, n);
}

static void blob(const char *k, variableLength *v) {
  unsigned long long h = 0xcbf29ce484222325ULL;
  DWORD i;
  if (v->size == 0 || !v->data) return;
  for (i = 0; i < v->size; i++) {
    h ^= v->data[i];
    h *= 0x100000001b3ULL;
  }
  printf("%s=%u:%016llx\n", k, v->size, h);
}

static void date(const char *k, dtr *d) {
  if (!d->wYear && !d->wMonth && !d->wDay && !d->wHour && !d->wMinute &&
      !d->wSecond && !d->wDayOfWeek)
    return;
  printf("%s=%u-%u-%u %u:%u:%u dow%u\n", k, d->wYear, d->wMonth, d->wDay,
         d->wHour, d->wMinute, d->wSecond, d->wDayOfWeek);
}

static int quiet_fd = -1;

/* libytnef prints diagnostics on stdout; keep them out of the dump */
static int quiet(int on) {
  int devnull;
  fflush(stdout);
  if (on) {
    quiet_fd = dup(1);
    devnull = open("/dev/null", O_WRONLY);
    if (quiet_fd < 0 || devnull < 0 || dup2(devnull, 1) < 0) return -1;
    close(devnull);
    return 0;
  }
  devnull = dup2(quiet_fd, 1);
  close(quiet_fd);
  return devnull < 0 ? -1 : 0;
}

static void rtf(MAPIProps *p) {
  variableLength *v = MAPIFindProperty(p, PROP_TAG(PT_BINARY, PR_RTF_COMPRESSED));
  variableLength out;
  int size = 0;
  BYTE *r;
  if (v == MAPI_UNDEFINED) return;
  if (quiet(1) < 0) return;
  r = DecompressRTF(v, &size);
  if (quiet(0) < 0) return;
  if (!r) {
    printf("rtf=error\n");
    return;
  }
  out.data = r;
  out.size = size;
  if (size == 0) printf("rtf=0:%016llx\n", 0xcbf29ce484222325ULL);
  else blob("rtf", &out);
  free(r);
}

static void attach(const char *name, Attachment *a) {
  printf("[%s]\n", name);
  if (a->RenderData.atyp) printf("atyp=%u\n", a->RenderData.atyp);
  vl("title", &a->Title);
  blob("data", &a->FileData);
  blob("icon", &a->IconData);
  date("create", &a->CreateDate);
  date("modify", &a->ModifyDate);
}

int main(int argc, char **argv) {
  TNEFStruct t;
  Attachment *a;
  int rc, n = 0;
  char name[32];

  if (argc != 2) {
    fprintf(stderr, "usage: tnefdump <file>\n");
    return 2;
  }
  TNEFInitialize(&t);
  if (quiet(1) < 0) return 2;
  rc = TNEFParseFile(argv[1], &t);
  if (quiet(0) < 0) return 2;
  printf("rc=%d\n", rc);
  if (rc == 0) {
    fixed("version", t.version, sizeof t.version);
    vl("from", &t.from);
    vl("subject", &t.subject);
    vl("body", &t.body);
    fixed("class", t.messageClass, sizeof t.messageClass);
    fixed("id", t.messageID, sizeof t.messageID);
    fixed("parent", t.parentID, sizeof t.parentID);
    fixed("priority", t.priority, sizeof t.priority);
    vl("codepage", &t.CodePage);
    vl("origclass", &t.OriginalMessageClass);
    date("sent", &t.dateSent);
    date("received", &t.dateReceived);
    date("modified", &t.dateModified);
    date("start", &t.DateStart);
    date("end", &t.DateEnd);
    rtf(&t.MapiProperties);
    attach("head", &t.starting_attach);
    for (a = t.starting_attach.next; a; a = a->next) {
      snprintf(name, sizeof name, "attachment %d", n++);
      attach(name, a);
    }
  }
  TNEFFree(&t);
  return 0;
}
