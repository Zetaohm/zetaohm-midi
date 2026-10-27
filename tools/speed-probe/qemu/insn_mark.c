/*
 * QEMU TCG plugin of the speed probe (docs/optimization-backlog.md, OPT-029).
 *
 * Counts the executed guest instructions. Each time the guest enters the marker function (plugin
 * argument mark=<address>), the plugin prints "MARK <count>" with the instructions executed before
 * it. scripts/speed-check.py pairs the marks with the bench names that the guest prints through
 * semihosting.
 *
 * Only the plugin API of QEMU 4.2 and later is used (Ubuntu 24.04 has QEMU 8.2): one callback for
 * each executed translation block adds the instruction count of the block. A block runs to its
 * end because the probe takes no interrupt; the marker function starts a block because it is a
 * branch target.
 *
 * With profile=<k>, the plugin also counts the instructions of each translation block between
 * mark 2k+1 and mark 2k+2 (the bench with index k, the calibration is index 0) and prints
 * "PROF <block address> <instructions>" at exit; the script gives them to functions.
 *
 *     qemu-system-arm -M mps2-an386 ... -plugin insn_mark.so,mark=0x1234[,profile=3] -d plugin
 */
#include <inttypes.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include <glib.h>
#include <qemu-plugin.h>

QEMU_PLUGIN_EXPORT int qemu_plugin_version = QEMU_PLUGIN_VERSION;

/* One translation block: its address and instruction count. */
struct block {
    uint64_t vaddr;
    uint64_t insns;
};

static uint64_t count;
static uint64_t mark_addr;
static uint64_t marks;
static int64_t profile = -1;
static GHashTable *prof; /* block address -> executed instructions in the profiled bench */

static void on_block(unsigned int vcpu, void *udata)
{
    struct block *b = udata;
    (void)vcpu;
    count += b->insns;
    if (prof && marks == (uint64_t)(2 * profile + 1)) {
        gpointer key = (gpointer)(uintptr_t)b->vaddr;
        uint64_t old = (uint64_t)(uintptr_t)g_hash_table_lookup(prof, key);
        g_hash_table_insert(prof, key, (gpointer)(uintptr_t)(old + b->insns));
    }
}

static void on_mark(unsigned int vcpu, void *udata)
{
    char line[64];
    snprintf(line, sizeof line, "MARK %" PRIu64 "\n", count);
    qemu_plugin_outs(line);
    marks++;
    on_block(vcpu, udata);
}

static void on_tb(qemu_plugin_id_t id, struct qemu_plugin_tb *tb)
{
    struct block *b = g_new(struct block, 1);
    b->vaddr = qemu_plugin_tb_vaddr(tb);
    b->insns = qemu_plugin_tb_n_insns(tb);
    /* Thumb: the symbol address has bit 0 set; the instruction address does not. */
    int is_mark = (b->vaddr | 1) == (mark_addr | 1);
    (void)id;
    qemu_plugin_register_vcpu_tb_exec_cb(tb, is_mark ? on_mark : on_block, QEMU_PLUGIN_CB_NO_REGS, b);
}

static void print_prof(gpointer key, gpointer value, gpointer udata)
{
    char line[64];
    (void)udata;
    snprintf(line, sizeof line, "PROF %#" PRIx64 " %" PRIu64 "\n", (uint64_t)(uintptr_t)key,
             (uint64_t)(uintptr_t)value);
    qemu_plugin_outs(line);
}

static void on_plugin_exit(qemu_plugin_id_t id, void *udata)
{
    char line[64];
    (void)id;
    (void)udata;
    snprintf(line, sizeof line, "TOTAL %" PRIu64 "\n", count);
    qemu_plugin_outs(line);
    if (prof) {
        g_hash_table_foreach(prof, print_prof, NULL);
    }
}

QEMU_PLUGIN_EXPORT int qemu_plugin_install(qemu_plugin_id_t id, const qemu_info_t *info, int argc,
                                           char **argv)
{
    (void)info;
    for (int i = 0; i < argc; i++) {
        if (strncmp(argv[i], "mark=", 5) == 0) {
            mark_addr = strtoull(argv[i] + 5, NULL, 0);
        } else if (strncmp(argv[i], "profile=", 8) == 0) {
            profile = strtoll(argv[i] + 8, NULL, 0);
            prof = g_hash_table_new(g_direct_hash, g_direct_equal);
        }
    }
    if (mark_addr == 0) {
        fprintf(stderr, "insn_mark: give mark=<address>\n");
        return -1;
    }
    qemu_plugin_register_vcpu_tb_trans_cb(id, on_tb);
    qemu_plugin_register_atexit_cb(id, on_plugin_exit, NULL);
    return 0;
}
