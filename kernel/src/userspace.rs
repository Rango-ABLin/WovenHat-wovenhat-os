use core::arch::global_asm;

use crate::config::{MAX_ANONYMOUS_MAPPINGS, MAX_ELF_SEGMENTS};
use crate::paging;

pub use crate::config::MAX_ANONYMOUS_MAPPINGS;

const USER_REGION_START: u64 = 0x0000_4000_0000_0000;
const USER_STACK_OFFSET: u64 = 0x1f_0000;
const USER_CODE_SIZE: usize = 4096;

global_asm!(
    ".global wovenhat_user_program_start",
    ".global wovenhat_user_program_end",
    "wovenhat_user_program_start:",
    "mov eax, 16",
    "int 0x80",
    "cmp rax, -1",
    "je wovenhat_user_failure",
    "test rax, rax",
    "jz wovenhat_fork_child",
    "mov r12, rax",
    "wovenhat_fork_wait:",
    "mov eax, 5",
    "mov rdi, r12",
    "int 0x80",
    "cmp rax, -2",
    "jne wovenhat_fork_reaped",
    "mov eax, 7",
    "int 0x80",
    "jmp wovenhat_fork_wait",
    "wovenhat_fork_reaped:",
    "cmp rax, 42",
    "jne wovenhat_user_failure",
    "jmp wovenhat_fork_parent",
    "wovenhat_fork_child:",
    "mov edi, 42",
    "mov eax, 3",
    "int 0x80",
    "wovenhat_fork_parent:",
    "mov eax, 0",
    "xor edi, edi",
    "mov rsi, 0x4000001f0000",
    "mov edx, 1",
    "int 0x80",
    "cmp rax, 1",
    "jne wovenhat_user_failure",
    "cmp byte ptr [0x4000001f0000], 97",
    "jne wovenhat_user_failure",
    "mov eax, 1",
    "mov edi, 1",
    "lea rsi, [rip + wovenhat_user_message]",
    "mov edx, 27",
    "int 0x80",
    "mov eax, 1",
    "mov edi, 2",
    "lea rsi, [rip + wovenhat_user_message]",
    "xor edx, edx",
    "int 0x80",
    "test rax, rax",
    "jne wovenhat_user_failure",
    "mov eax, 2",
    "lea rdi, [rip + wovenhat_user_path]",
    "mov esi, 9",
    "int 0x80",
    "mov r12, rax",
    "mov eax, 0",
    "mov esi, 0x1f0000",
    "mov ebx, 0x40000",
    "shl rbx, 32",
    "or rsi, rbx",
    "mov rdi, r12",
    "mov edx, 64",
    "int 0x80",
    "mov r13, rax",
    "mov eax, 1",
    "mov edi, 1",
    "mov esi, 0x1f0000",
    "mov ebx, 0x40000",
    "shl rbx, 32",
    "or rsi, rbx",
    "mov eax, 6",
    "mov rdi, r12",
    "int 0x80",
    "mov eax, 8",
    "mov edi, 4096",
    "mov esi, 1",
    "int 0x80",
    "mov r14, rax",
    "mov r15d, 0x4e484154",
    "mov ebx, 0x574f5645",
    "shl rbx, 32",
    "or r15, rbx",
    "mov qword ptr [r14], r15",
    "cmp qword ptr [r14], r15",
    "jne wovenhat_user_failure",
    "mov eax, 9",
    "mov rdi, r14",
    "mov esi, 4096",
    "int 0x80",
    "test rax, rax",
    "jne wovenhat_user_failure",
    "mov eax, 13",
    "int 0x80",
    "cmp rax, 1000",
    "jne wovenhat_user_failure",
    "mov eax, 14",
    "int 0x80",
    "cmp rax, 1000",
    "jne wovenhat_user_failure",
    "mov eax, 4",
    "int 0x80",
    "xor edi, edi",
    "mov eax, 3",
    "int 0x80",
    "wovenhat_user_failure:",
    "mov edi, 1",
    "mov eax, 3",
    "int 0x80",
    "2:",
    "jmp 2b",
    "wovenhat_user_message:",
    ".ascii \"[USER] syscall I/O online!\\n\"",
    "wovenhat_user_path:",
    ".ascii \"/etc/motd\"",
    "wovenhat_user_program_end:",
);

global_asm!(
    ".section .rodata.wovenhat_exec_stub, \"a\"",
    ".global wovenhat_exec_program_start",
    ".global wovenhat_exec_program_end",
    "wovenhat_exec_program_start:",
    "mov eax, 15",
    "lea rdi, [rip + wovenhat_exec_path]",
    "mov esi, 13",
    "int 0x80",
    "mov edi, 1",
    "mov eax, 3",
    "int 0x80",
    "1:",
    "jmp 1b",
    "wovenhat_exec_path:",
    ".ascii \"/bin/selftest\"",
    "wovenhat_exec_program_end:",
    ".previous",
);

// Userspace init: read argc/argv from the stack, print a banner, exit 0.
global_asm!(
    ".section .rodata.wovenhat_init_stub, \"a\"",
    ".global wovenhat_init_program_start",
    ".global wovenhat_init_program_end",
    "wovenhat_init_program_start:",
    // write banner
    "mov eax, 1",
    "mov edi, 1",
    "lea rsi, [rip + wovenhat_init_banner]",
    "mov edx, 28",
    "int 0x80",
    // exec("/bin/sh")
    "mov eax, 15",
    "lea rdi, [rip + wovenhat_init_sh]",
    "mov esi, 7",
    "int 0x80",
    // exec failed — exit 1
    "mov edi, 1",
    "mov eax, 3",
    "int 0x80",
    "1:",
    "jmp 1b",
    "wovenhat_init_banner:",
    ".ascii \"[INIT] userspace online\\n\"",
    "wovenhat_init_sh:",
    ".ascii \"/bin/sh\"",
    "wovenhat_init_program_end:",
    ".previous",
);

// Userspace shell (/bin/sh): interactive line editor + builtins + fork/exec.
global_asm!(
    ".section .rodata.wovenhat_sh_stub, \"a\"",
    ".global wovenhat_sh_program_start",
    ".global wovenhat_sh_program_end",
    "wovenhat_sh_program_start:",
    // mmap(4096, writable=1) -> line buffer in r14
    "mov eax, 8",
    "mov edi, 4096",
    "mov esi, 1",
    "int 0x80",
    "cmp rax, -1",
    "je wovenhat_sh_die",
    "mov r14, rax",
    // banner
    "mov eax, 1",
    "mov edi, 1",
    "lea rsi, [rip + wovenhat_sh_banner]",
    "mov edx, 26",
    "int 0x80",
    "wovenhat_sh_loop:",
    // prompt
    "mov eax, 1",
    "mov edi, 1",
    "lea rsi, [rip + wovenhat_sh_prompt]",
    "mov edx, 4",
    "int 0x80",
    // read line into [r14], max 120 chars, r15 = length
    "xor r15, r15",
    "wovenhat_sh_read:",
    // use byte at [r14+200] as scratch (writable mmap page)
    "mov eax, 0",
    "xor edi, edi",
    "lea rsi, [r14 + 200]",
    "mov edx, 1",
    "int 0x80",
    "cmp rax, -1",
    "je wovenhat_sh_read_yield",
    "test rax, rax",
    "jz wovenhat_sh_read_yield",
    "movzx eax, byte ptr [r14 + 200]",
    // newline?
    "cmp al, 10",
    "je wovenhat_sh_got_line",
    "cmp al, 13",
    "je wovenhat_sh_got_line",
    // backspace / del
    "cmp al, 8",
    "je wovenhat_sh_bksp",
    "cmp al, 127",
    "je wovenhat_sh_bksp",
    // ignore other controls
    "cmp al, 32",
    "jb wovenhat_sh_read",
    "cmp r15, 120",
    "jae wovenhat_sh_read",
    "mov byte ptr [r14 + r15], al",
    "inc r15",
    // echo
    "mov eax, 1",
    "mov edi, 1",
    "lea rsi, [r14 + 200]",
    "mov edx, 1",
    "int 0x80",
    "jmp wovenhat_sh_read",
    "wovenhat_sh_bksp:",
    "test r15, r15",
    "jz wovenhat_sh_read",
    "dec r15",
    "mov eax, 1",
    "mov edi, 1",
    "lea rsi, [rip + wovenhat_sh_bksp_seq]",
    "mov edx, 3",
    "int 0x80",
    "jmp wovenhat_sh_read",
    "wovenhat_sh_read_yield:",
    "mov eax, 7",
    "int 0x80",
    "jmp wovenhat_sh_read",
    "wovenhat_sh_got_line:",
    "mov byte ptr [r14 + 600], 0",
    // echo newline
    "mov eax, 1",
    "mov edi, 1",
    "lea rsi, [rip + wovenhat_sh_nl]",
    "mov edx, 1",
    "int 0x80",
    "mov byte ptr [r14 + r15], 0",
    "test r15, r15",
    "jz wovenhat_sh_loop",
    // builtins: exit / quit
    "lea rdi, [rip + wovenhat_sh_cmd_exit]",
    "call wovenhat_sh_eq",
    "jc wovenhat_sh_do_exit",
    "lea rdi, [rip + wovenhat_sh_cmd_quit]",
    "call wovenhat_sh_eq",
    "jc wovenhat_sh_do_exit",
    // help
    "lea rdi, [rip + wovenhat_sh_cmd_help]",
    "call wovenhat_sh_eq",
    "jc wovenhat_sh_do_help",
    // echo ...
    "lea rdi, [rip + wovenhat_sh_cmd_echo]",
    "call wovenhat_sh_prefix",
    "jc wovenhat_sh_do_echo",
    // pwd
    "lea rdi, [rip + wovenhat_sh_cmd_pwd]",
    "call wovenhat_sh_eq",
    "jc wovenhat_sh_do_pwd",
    // cd <path>
    "lea rdi, [rip + wovenhat_sh_cmd_cd]",
    "call wovenhat_sh_prefix",
    "jc wovenhat_sh_do_cd",
    // cat <path>
    "lea rdi, [rip + wovenhat_sh_cmd_cat]",
    "call wovenhat_sh_prefix",
    "jc wovenhat_sh_do_cat",
    // ls [path]
    "lea rdi, [rip + wovenhat_sh_cmd_ls]",
    "call wovenhat_sh_prefix",
    "jc wovenhat_sh_do_ls",
    // mkdir <path>
    "lea rdi, [rip + wovenhat_sh_cmd_mkdir]",
    "call wovenhat_sh_prefix",
    "jc wovenhat_sh_do_mkdir",
    // rm <path>
    "lea rdi, [rip + wovenhat_sh_cmd_rm]",
    "call wovenhat_sh_prefix",
    "jc wovenhat_sh_do_rm",
    // mv <old> <new>
    "lea rdi, [rip + wovenhat_sh_cmd_mv]",
    "call wovenhat_sh_prefix",
    "jc wovenhat_sh_do_mv",
    // getppid
    "lea rdi, [rip + wovenhat_sh_cmd_getppid]",
    "call wovenhat_sh_eq",
    "jc wovenhat_sh_do_getppid",
    // ticks
    "lea rdi, [rip + wovenhat_sh_cmd_ticks]",
    "call wovenhat_sh_eq",
    "jc wovenhat_sh_do_ticks",
    // clear
    "lea rdi, [rip + wovenhat_sh_cmd_clear]",
    "call wovenhat_sh_eq",
    "jc wovenhat_sh_do_clear",
    // yield
    "lea rdi, [rip + wovenhat_sh_cmd_yield]",
    "call wovenhat_sh_eq",
    "jc wovenhat_sh_do_yield",
    // which <name>
    "lea rdi, [rip + wovenhat_sh_cmd_which]",
    "call wovenhat_sh_prefix",
    "jc wovenhat_sh_do_which",
    // wait <pid>
    "lea rdi, [rip + wovenhat_sh_cmd_wait]",
    "call wovenhat_sh_prefix",
    "jc wovenhat_sh_do_wait",
    // test -f/-d path
    "lea rdi, [rip + wovenhat_sh_cmd_test]",
    "call wovenhat_sh_prefix",
    "jc wovenhat_sh_do_test",
    // null command
    "lea rdi, [rip + wovenhat_sh_cmd_colon]",
    "call wovenhat_sh_eq",
    "jc wovenhat_sh_loop",
    // sleep <ticks>
    "lea rdi, [rip + wovenhat_sh_cmd_sleep]",
    "call wovenhat_sh_prefix",
    "jc wovenhat_sh_do_sleep",
    // getpid
    "lea rdi, [rip + wovenhat_sh_cmd_getpid]",
    "call wovenhat_sh_eq",
    "jc wovenhat_sh_do_getpid",
    // kill <pid>
    "lea rdi, [rip + wovenhat_sh_cmd_kill]",
    "call wovenhat_sh_prefix",
    "jc wovenhat_sh_do_kill",
    // background: trailing &
    "call wovenhat_sh_find_bg",
    "jc wovenhat_sh_mark_bg",
    "wovenhat_sh_after_bg:",
    // pipeline? up to 3 stages
    "call wovenhat_sh_count_pipes",
    "cmp eax, 2",
    "jae wovenhat_sh_do_pipeline3",
    "call wovenhat_sh_find_pipe",
    "jc wovenhat_sh_do_pipeline",
    // redirect append: cmd >> file
    "call wovenhat_sh_find_append",
    "jc wovenhat_sh_do_redir_append",
    // redirect out: cmd > file
    "call wovenhat_sh_find_gt",
    "jc wovenhat_sh_do_redir_out",
    // redirect in: cmd < file
    "call wovenhat_sh_find_lt",
    "jc wovenhat_sh_do_redir_in",
    // otherwise treat line as path to exec
    "jmp wovenhat_sh_do_exec",
    "wovenhat_sh_do_exit:",
    "xor edi, edi",
    "mov eax, 3",
    "int 0x80",
    "wovenhat_sh_do_help:",
    "mov eax, 1",
    "mov edi, 1",
    "lea rsi, [rip + wovenhat_sh_help]",
    "mov edx, 112",
    "int 0x80",
    "jmp wovenhat_sh_loop",
    "wovenhat_sh_do_echo:",
    // skip "echo" and optional space
    "mov rcx, r15",
    "cmp rcx, 5",
    "jbe wovenhat_sh_echo_nl",
    "lea rsi, [r14 + 5]",
    "cmp byte ptr [rsi], 32",
    "jne 1f",
    "inc rsi",
    "1:",
    "mov rdx, r14",
    "add rdx, r15",
    "sub rdx, rsi",
    "test rdx, rdx",
    "jz wovenhat_sh_echo_nl",
    "mov eax, 1",
    "mov edi, 1",
    "int 0x80",
    "wovenhat_sh_echo_nl:",
    "mov eax, 1",
    "mov edi, 1",
    "lea rsi, [rip + wovenhat_sh_nl]",
    "mov edx, 1",
    "int 0x80",
    "jmp wovenhat_sh_loop",
    "wovenhat_sh_do_pwd:",
    "mov eax, 21",
    "mov rdi, r14",
    "mov esi, 120",
    "int 0x80",
    "cmp rax, -1",
    "je wovenhat_sh_loop",
    "mov rdx, rax",
    "mov eax, 1",
    "mov edi, 1",
    "mov rsi, r14",
    "int 0x80",
    "mov eax, 1",
    "mov edi, 1",
    "lea rsi, [rip + wovenhat_sh_nl]",
    "mov edx, 1",
    "int 0x80",
    "jmp wovenhat_sh_loop",
    "wovenhat_sh_do_cd:",
    // path starts after "cd"
    "cmp r15, 3",
    "jbe wovenhat_sh_cd_root",
    "lea rdi, [r14 + 2]",
    "cmp byte ptr [rdi], 32",
    "jne 1f",
    "inc rdi",
    "1:",
    // path length
    "mov rsi, r14",
    "add rsi, r15",
    "sub rsi, rdi",
    "mov eax, 20",
    "int 0x80",
    "cmp rax, -1",
    "jne wovenhat_sh_loop",
    "mov eax, 1",
    "mov edi, 1",
    "lea rsi, [rip + wovenhat_sh_cd_fail]",
    "mov edx, 9",
    "int 0x80",
    "jmp wovenhat_sh_loop",
    "wovenhat_sh_cd_root:",
    "mov eax, 20",
    "lea rdi, [rip + wovenhat_sh_slash]",
    "mov esi, 1",
    "int 0x80",
    "jmp wovenhat_sh_loop",
    "wovenhat_sh_do_cat:",
    // path after "cat"
    "cmp r15, 4",
    "jbe wovenhat_sh_cat_usage",
    "lea rdi, [r14 + 3]",
    "cmp byte ptr [rdi], 32",
    "jne 1f",
    "inc rdi",
    "1:",
    // rsi = path length
    "mov rsi, r14",
    "add rsi, r15",
    "sub rsi, rdi",
    "test rsi, rsi",
    "jz wovenhat_sh_cat_usage",
    // open(path)
    "mov eax, 2",
    "int 0x80",
    "cmp rax, -1",
    "je wovenhat_sh_cat_fail",
    "mov r12, rax",
    // read/write loop using scratch at r14+256
    "wovenhat_sh_cat_loop:",
    "mov eax, 0",
    "mov rdi, r12",
    "lea rsi, [r14 + 256]",
    "mov edx, 128",
    "int 0x80",
    "cmp rax, -1",
    "je wovenhat_sh_cat_close",
    "test rax, rax",
    "jz wovenhat_sh_cat_close",
    "mov rdx, rax",
    "mov eax, 1",
    "mov edi, 1",
    "lea rsi, [r14 + 256]",
    "int 0x80",
    "jmp wovenhat_sh_cat_loop",
    "wovenhat_sh_cat_close:",
    "mov eax, 6",
    "mov rdi, r12",
    "int 0x80",
    "mov eax, 1",
    "mov edi, 1",
    "lea rsi, [rip + wovenhat_sh_nl]",
    "mov edx, 1",
    "int 0x80",
    "jmp wovenhat_sh_loop",
    "wovenhat_sh_cat_usage:",
    "mov eax, 1",
    "mov edi, 1",
    "lea rsi, [rip + wovenhat_sh_cat_usage_msg]",
    "mov edx, 14",
    "int 0x80",
    "jmp wovenhat_sh_loop",
    "wovenhat_sh_cat_fail:",
    "mov eax, 1",
    "mov edi, 1",
    "lea rsi, [rip + wovenhat_sh_cat_fail_msg]",
    "mov edx, 10",
    "int 0x80",
    "jmp wovenhat_sh_loop",

    "wovenhat_sh_do_ls:",
    // default path = cwd into r14+300; or arg path
    "cmp r15, 2",
    "ja wovenhat_sh_ls_arg",
    // getcwd
    "mov eax, 21",
    "lea rdi, [r14 + 300]",
    "mov esi, 100",
    "int 0x80",
    "cmp rax, -1",
    "je wovenhat_sh_ls_fail",
    "mov r13, rax",
    "lea r12, [r14 + 300]",
    "jmp wovenhat_sh_ls_go",
    "wovenhat_sh_ls_arg:",
    "lea r12, [r14 + 2]",
    "cmp byte ptr [r12], 32",
    "jne 1f",
    "inc r12",
    "1:",
    "mov r13, r14",
    "add r13, r15",
    "sub r13, r12",
    "wovenhat_sh_ls_go:",
    "xor r15, r15",
    "wovenhat_sh_ls_loop:",
    // arg1 = path_len | (index<<16)
    "mov rsi, r13",
    "mov rax, r15",
    "shl rax, 16",
    "or rsi, rax",
    "mov eax, 18",
    "mov rdi, r12",
    "lea rdx, [r14 + 400]",
    "int 0x80",
    "cmp rax, -1",
    "je wovenhat_sh_loop",
    // print kind prefix
    "mov rbx, rax",
    "shr rax, 8",
    "and eax, 0xff",
    "cmp al, 1",
    "jne 2f",
    "mov eax, 1",
    "mov edi, 1",
    "lea rsi, [rip + wovenhat_sh_ls_d]",
    "mov edx, 2",
    "int 0x80",
    "jmp 3f",
    "2:",
    "mov eax, 1",
    "mov edi, 1",
    "lea rsi, [rip + wovenhat_sh_ls_f]",
    "mov edx, 2",
    "int 0x80",
    "3:",
    // print name (low 8 bits of return = length)
    "mov rdx, rbx",
    "and edx, 0xff",
    "mov eax, 1",
    "mov edi, 1",
    "lea rsi, [r14 + 400]",
    "int 0x80",
    "mov eax, 1",
    "mov edi, 1",
    "lea rsi, [rip + wovenhat_sh_nl]",
    "mov edx, 1",
    "int 0x80",
    "inc r15",
    "cmp r15, 64",
    "jb wovenhat_sh_ls_loop",
    "jmp wovenhat_sh_loop",
    "wovenhat_sh_ls_fail:",
    "mov eax, 1",
    "mov edi, 1",
    "lea rsi, [rip + wovenhat_sh_ls_err]",
    "mov edx, 9",
    "int 0x80",
    "jmp wovenhat_sh_loop",
    "wovenhat_sh_do_mkdir:",
    "cmp r15, 6",
    "jbe wovenhat_sh_mkdir_usage",
    "lea rdi, [r14 + 5]",
    "cmp byte ptr [rdi], 32",
    "jne 1f",
    "inc rdi",
    "1:",
    "mov rsi, r14",
    "add rsi, r15",
    "sub rsi, rdi",
    "test rsi, rsi",
    "jz wovenhat_sh_mkdir_usage",
    "mov eax, 19",
    "int 0x80",
    "cmp rax, -1",
    "je wovenhat_sh_mkdir_fail",
    "jmp wovenhat_sh_loop",
    "wovenhat_sh_mkdir_usage:",
    "mov eax, 1",
    "mov edi, 1",
    "lea rsi, [rip + wovenhat_sh_mkdir_usage_msg]",
    "mov edx, 16",
    "int 0x80",
    "jmp wovenhat_sh_loop",
    "wovenhat_sh_mkdir_fail:",
    "mov eax, 1",
    "mov edi, 1",
    "lea rsi, [rip + wovenhat_sh_mkdir_fail_msg]",
    "mov edx, 12",
    "int 0x80",
    "jmp wovenhat_sh_loop",
    // Find '|': CF set, r8 = index of '|'. Clobbers rax/rcx.



    "wovenhat_sh_do_mv:",
    // mv old new — two args
    "cmp r15, 4",
    "jbe wovenhat_sh_mv_usage",
    "lea r8, [r14 + 2]",
    "cmp byte ptr [r8], 32",
    "jne 1f",
    "inc r8",
    "1:",
    // find space between old and new
    "xor rcx, rcx",
    "2:",
    "movzx eax, byte ptr [r8 + rcx]",
    "cmp al, 0",
    "je wovenhat_sh_mv_usage",
    "cmp al, 32",
    "je 3f",
    "inc rcx",
    "jmp 2b",
    "3:",
    "mov r9, rcx",
    // new path
    "lea r10, [r8 + rcx]",
    "4:",
    "cmp byte ptr [r10], 32",
    "jne 5f",
    "inc r10",
    "jmp 4b",
    "5:",
    "mov r11, r14",
    "add r11, r15",
    "sub r11, r10",
    "test r11, r11",
    "jz wovenhat_sh_mv_usage",
    // rename: arg0=old, arg1=old_len|(new_len<<16), arg2=new
    "mov eax, 30",
    "mov rdi, r8",
    "mov rsi, r9",
    "mov rax, r11",
    "shl rax, 16",
    "or rsi, rax",
    "mov rdx, r10",
    "mov eax, 30",
    "int 0x80",
    "cmp rax, -1",
    "je wovenhat_sh_mv_fail",
    "jmp wovenhat_sh_loop",
    "wovenhat_sh_mv_usage:",
    "mov eax, 1",
    "mov edi, 1",
    "lea rsi, [rip + wovenhat_sh_mv_usage_msg]",
    "mov edx, 16",
    "int 0x80",
    "jmp wovenhat_sh_loop",
    "wovenhat_sh_mv_fail:",
    "mov eax, 1",
    "mov edi, 1",
    "lea rsi, [rip + wovenhat_sh_mv_fail_msg]",
    "mov edx, 9",
    "int 0x80",
    "jmp wovenhat_sh_loop",
    "wovenhat_sh_do_getppid:",
    "mov eax, 25",
    "int 0x80",
    "call wovenhat_sh_print_u",
    "mov eax, 1",
    "mov edi, 1",
    "lea rsi, [rip + wovenhat_sh_nl]",
    "mov edx, 1",
    "int 0x80",
    "jmp wovenhat_sh_loop",
    "wovenhat_sh_do_ticks:",
    "mov eax, 31",
    "int 0x80",
    "call wovenhat_sh_print_u",
    "mov eax, 1",
    "mov edi, 1",
    "lea rsi, [rip + wovenhat_sh_nl]",
    "mov edx, 1",
    "int 0x80",
    "jmp wovenhat_sh_loop",
    "wovenhat_sh_do_clear:",
    "mov eax, 1",
    "mov edi, 1",
    "lea rsi, [rip + wovenhat_sh_clear_seq]",
    "mov edx, 7",
    "int 0x80",
    "jmp wovenhat_sh_loop",
    "wovenhat_sh_do_yield:",
    "mov eax, 7",
    "int 0x80",
    "jmp wovenhat_sh_loop",
    "wovenhat_sh_do_which:",
    "cmp r15, 6",
    "jbe wovenhat_sh_which_usage",
    "lea r8, [r14 + 5]",
    "cmp byte ptr [r8], 32",
    "jne 1f",
    "inc r8",
    "1:",
    // build /bin/<name> at r14+300
    "lea rdi, [r14 + 300]",
    "mov byte ptr [rdi], 47",
    "mov byte ptr [rdi+1], 98",
    "mov byte ptr [rdi+2], 105",
    "mov byte ptr [rdi+3], 110",
    "mov byte ptr [rdi+4], 47",
    "lea rsi, [rdi + 5]",
    "xor rcx, rcx",
    "2:",
    "movzx eax, byte ptr [r8 + rcx]",
    "cmp al, 0",
    "je 3f",
    "cmp al, 32",
    "je 3f",
    "mov [rsi + rcx], al",
    "inc rcx",
    "cmp rcx, 64",
    "jb 2b",
    "3:",
    "mov byte ptr [rsi + rcx], 0",
    "add rcx, 5",
    // stat path
    "mov eax, 17",
    "mov rdi, rdi",
    "mov rsi, rcx",
    "int 0x80",
    "cmp rax, -1",
    "je wovenhat_sh_which_fail",
    "mov eax, 1",
    "mov edi, 1",
    "lea rsi, [r14 + 300]",
    "mov rdx, rcx",
    "int 0x80",
    "mov eax, 1",
    "mov edi, 1",
    "lea rsi, [rip + wovenhat_sh_nl]",
    "mov edx, 1",
    "int 0x80",
    "jmp wovenhat_sh_loop",
    "wovenhat_sh_which_usage:",
    "mov eax, 1",
    "mov edi, 1",
    "lea rsi, [rip + wovenhat_sh_which_usage_msg]",
    "mov edx, 16",
    "int 0x80",
    "jmp wovenhat_sh_loop",
    "wovenhat_sh_which_fail:",
    "mov eax, 1",
    "mov edi, 1",
    "lea rsi, [rip + wovenhat_sh_which_fail_msg]",
    "mov edx, 12",
    "int 0x80",
    "jmp wovenhat_sh_loop",

    "wovenhat_sh_do_wait:",
    "cmp r15, 5",
    "jbe wovenhat_sh_wait_usage",
    "lea rsi, [r14 + 4]",
    "cmp byte ptr [rsi], 32",
    "jne 1f",
    "inc rsi",
    "1:",
    "xor eax, eax",
    "xor ecx, ecx",
    "2:",
    "movzx edx, byte ptr [rsi + rcx]",
    "cmp dl, 0",
    "je 3f",
    "cmp dl, 32",
    "je 3f",
    "sub dl, 48",
    "cmp dl, 9",
    "ja wovenhat_sh_wait_usage",
    "imul eax, eax, 10",
    "add eax, edx",
    "inc rcx",
    "jmp 2b",
    "3:",
    "mov r12, rax",
    "wovenhat_sh_wait_loop:",
    "mov eax, 5",
    "mov rdi, r12",
    "int 0x80",
    "cmp rax, -2",
    "jne wovenhat_sh_loop",
    "mov eax, 7",
    "int 0x80",
    "jmp wovenhat_sh_wait_loop",
    "wovenhat_sh_wait_usage:",
    "mov eax, 1",
    "mov edi, 1",
    "lea rsi, [rip + wovenhat_sh_wait_usage_msg]",
    "mov edx, 15",
    "int 0x80",
    "jmp wovenhat_sh_loop",
    "wovenhat_sh_do_test:",
    // test -f path  or test -d path
    "cmp r15, 6",
    "jbe wovenhat_sh_test_fail",
    "lea rsi, [r14 + 4]",
    "cmp byte ptr [rsi], 32",
    "jne 1f",
    "inc rsi",
    "1:",
    "cmp byte ptr [rsi], 45",
    "jne wovenhat_sh_test_fail",
    "movzx eax, byte ptr [rsi + 1]",
    "mov r8, rax",
    // path after flag
    "lea rdi, [rsi + 2]",
    "cmp byte ptr [rdi], 32",
    "jne 2f",
    "inc rdi",
    "2:",
    "mov rsi, r14",
    "add rsi, r15",
    "sub rsi, rdi",
    "test rsi, rsi",
    "jz wovenhat_sh_test_fail",
    "mov eax, 17",
    "int 0x80",
    "cmp rax, -1",
    "je wovenhat_sh_test_fail",
    // kind in low bits of packed stat - check syscall packing
    // bits[7:0] = kind (0=file, 1=directory)
    "mov rcx, rax",
    "and ecx, 0xff",
    "cmp r8b, 102",
    "je 3f",
    "cmp r8b, 100",
    "je 4f",
    "jmp wovenhat_sh_test_fail",
    "3:",
    // -f file
    "test ecx, ecx",
    "jnz wovenhat_sh_test_fail",
    "jmp wovenhat_sh_loop",
    "4:",
    // -d dir
    "cmp ecx, 1",
    "jne wovenhat_sh_test_fail",
    "jmp wovenhat_sh_loop",
    "wovenhat_sh_test_fail:",
    "mov eax, 1",
    "mov edi, 1",
    "lea rsi, [rip + wovenhat_sh_test_fail_msg]",
    "mov edx, 8",
    "int 0x80",
    "jmp wovenhat_sh_loop",
    "wovenhat_sh_do_rm:",
    "cmp r15, 3",
    "jbe wovenhat_sh_rm_usage",
    "lea rdi, [r14 + 2]",
    "cmp byte ptr [rdi], 32",
    "jne 1f",
    "inc rdi",
    "1:",
    "mov rsi, r14",
    "add rsi, r15",
    "sub rsi, rdi",
    "mov eax, 28",
    "int 0x80",
    "cmp rax, -1",
    "je wovenhat_sh_rm_fail",
    "jmp wovenhat_sh_loop",
    "wovenhat_sh_rm_usage:",
    "mov eax, 1",
    "mov edi, 1",
    "lea rsi, [rip + wovenhat_sh_rm_usage_msg]",
    "mov edx, 13",
    "int 0x80",
    "jmp wovenhat_sh_loop",
    "wovenhat_sh_rm_fail:",
    "mov eax, 1",
    "mov edi, 1",
    "lea rsi, [rip + wovenhat_sh_rm_fail_msg]",
    "mov edx, 9",
    "int 0x80",
    "jmp wovenhat_sh_loop",
    "wovenhat_sh_do_sleep:",
    "cmp r15, 6",
    "jbe wovenhat_sh_sleep_usage",
    "lea rsi, [r14 + 5]",
    "cmp byte ptr [rsi], 32",
    "jne 1f",
    "inc rsi",
    "1:",
    "xor eax, eax",
    "xor ecx, ecx",
    "2:",
    "movzx edx, byte ptr [rsi + rcx]",
    "cmp dl, 0",
    "je 3f",
    "cmp dl, 32",
    "je 3f",
    "sub dl, 48",
    "cmp dl, 9",
    "ja wovenhat_sh_sleep_usage",
    "imul eax, eax, 10",
    "add eax, edx",
    "inc rcx",
    "jmp 2b",
    "3:",
    "mov edi, eax",
    "mov eax, 29",
    "int 0x80",
    "jmp wovenhat_sh_loop",
    "wovenhat_sh_sleep_usage:",
    "mov eax, 1",
    "mov edi, 1",
    "lea rsi, [rip + wovenhat_sh_sleep_usage_msg]",
    "mov edx, 16",
    "int 0x80",
    "jmp wovenhat_sh_loop",
    "wovenhat_sh_do_getpid:",
    "mov eax, 4",
    "int 0x80",
    "call wovenhat_sh_print_u",
    "mov eax, 1",
    "mov edi, 1",
    "lea rsi, [rip + wovenhat_sh_nl]",
    "mov edx, 1",
    "int 0x80",
    "jmp wovenhat_sh_loop",
    // print unsigned rax as decimal using buffer at r14+500
    "wovenhat_sh_print_u:",
    "push rbx",
    "push rcx",
    "push rdx",
    "lea rbx, [r14 + 520]",
    "mov byte ptr [rbx], 0",
    "mov rcx, rax",
    "mov rdi, 10",
    "1:",
    "xor edx, edx",
    "mov rax, rcx",
    "div rdi",
    "mov rcx, rax",
    "add dl, 48",
    "dec rbx",
    "mov [rbx], dl",
    "test rcx, rcx",
    "jnz 1b",
    "mov rsi, rbx",
    "lea rdx, [r14 + 520]",
    "sub rdx, rsi",
    "mov eax, 1",
    "mov edi, 1",
    "int 0x80",
    "pop rdx",
    "pop rcx",
    "pop rbx",
    "ret",
    // find ">>"
    "wovenhat_sh_find_append:",
    "xor rcx, rcx",
    "1:",
    "mov rdx, r15",
    "sub rdx, 1",
    "cmp rcx, rdx",
    "jae 2f",
    "cmp byte ptr [r14 + rcx], 62",
    "jne 3f",
    "cmp byte ptr [r14 + rcx + 1], 62",
    "jne 3f",
    "mov r8, rcx",
    "stc",
    "ret",
    "3:",
    "inc rcx",
    "jmp 1b",
    "2:",
    "clc",
    "ret",
    "wovenhat_sh_do_redir_append:",
    // like redir_out but seek to end after open
    "mov r9, r8",
    "1:",
    "test r9, r9",
    "jz wovenhat_sh_redir_fail",
    "dec r9",
    "cmp byte ptr [r14 + r9], 32",
    "je 1b",
    "inc r9",
    "test r9, r9",
    "jz wovenhat_sh_redir_fail",
    "lea r10, [r14 + r8 + 2]",
    "2:",
    "cmp byte ptr [r10], 32",
    "jne 3f",
    "inc r10",
    "jmp 2b",
    "3:",
    "mov r11, r14",
    "add r11, r15",
    "sub r11, r10",
    "test r11, r11",
    "jz wovenhat_sh_redir_fail",
    "mov eax, 16",
    "int 0x80",
    "cmp rax, -1",
    "je wovenhat_sh_redir_fail",
    "test rax, rax",
    "jnz wovenhat_sh_redir_wait",
    "mov eax, 2",
    "mov rdi, r10",
    "mov rsi, r11",
    "int 0x80",
    "cmp rax, -1",
    "je 9f",
    "mov r12, rax",
    // lseek to large offset — seek clamps to end
    "mov eax, 27",
    "mov rdi, r12",
    "mov rsi, 0x7fffffff",
    "int 0x80",
    "mov eax, 24",
    "mov rdi, r12",
    "mov esi, 1",
    "int 0x80",
    "mov eax, 6",
    "mov rdi, r12",
    "int 0x80",
    "mov byte ptr [r14 + r9], 0",
    "mov eax, 15",
    "mov rdi, r14",
    "mov rsi, r9",
    "int 0x80",
    "9:",
    "mov edi, 1",
    "mov eax, 3",
    "int 0x80",
    "wovenhat_sh_do_kill:",
    "cmp r15, 5",
    "jbe wovenhat_sh_kill_usage",
    "lea rsi, [r14 + 4]",
    "cmp byte ptr [rsi], 32",
    "jne 1f",
    "inc rsi",
    "1:",
    // parse decimal pid into rdi
    "xor eax, eax",
    "xor ecx, ecx",
    "2:",
    "movzx edx, byte ptr [rsi + rcx]",
    "cmp dl, 0",
    "je 3f",
    "cmp dl, 32",
    "je 3f",
    "sub dl, 48",
    "cmp dl, 9",
    "ja wovenhat_sh_kill_usage",
    "imul eax, eax, 10",
    "add eax, edx",
    "inc rcx",
    "jmp 2b",
    "3:",
    "mov edi, eax",
    "mov esi, 15",
    "mov eax, 26",
    "int 0x80",
    "cmp rax, -1",
    "je wovenhat_sh_kill_fail",
    "jmp wovenhat_sh_loop",
    "wovenhat_sh_kill_usage:",
    "mov eax, 1",
    "mov edi, 1",
    "lea rsi, [rip + wovenhat_sh_kill_usage_msg]",
    "mov edx, 15",
    "int 0x80",
    "jmp wovenhat_sh_loop",
    "wovenhat_sh_kill_fail:",
    "mov eax, 1",
    "mov edi, 1",
    "lea rsi, [rip + wovenhat_sh_kill_fail_msg]",
    "mov edx, 11",
    "int 0x80",
    "jmp wovenhat_sh_loop",
    // find '>' 
    "wovenhat_sh_find_gt:",
    "xor rcx, rcx",
    "1:",
    "cmp rcx, r15",
    "jae 2f",
    "cmp byte ptr [r14 + rcx], 62",
    "je 3f",
    "inc rcx",
    "jmp 1b",
    "2:",
    "clc",
    "ret",
    "3:",
    "mov r8, rcx",
    "stc",
    "ret",
    // find '<'
    "wovenhat_sh_find_lt:",
    "xor rcx, rcx",
    "1:",
    "cmp rcx, r15",
    "jae 2f",
    "cmp byte ptr [r14 + rcx], 60",
    "je 3f",
    "inc rcx",
    "jmp 1b",
    "2:",
    "clc",
    "ret",
    "3:",
    "mov r8, rcx",
    "stc",
    "ret",
    "wovenhat_sh_do_redir_out:",
    // left cmd in r14/r9, file path in r10/r11
    "mov r9, r8",
    "1:",
    "test r9, r9",
    "jz wovenhat_sh_redir_fail",
    "dec r9",
    "cmp byte ptr [r14 + r9], 32",
    "je 1b",
    "inc r9",
    "test r9, r9",
    "jz wovenhat_sh_redir_fail",
    "lea r10, [r14 + r8 + 1]",
    "2:",
    "cmp byte ptr [r10], 32",
    "jne 3f",
    "inc r10",
    "jmp 2b",
    "3:",
    "mov r11, r14",
    "add r11, r15",
    "sub r11, r10",
    "test r11, r11",
    "jz wovenhat_sh_redir_fail",
    "mov eax, 16",
    "int 0x80",
    "cmp rax, -1",
    "je wovenhat_sh_redir_fail",
    "test rax, rax",
    "jnz wovenhat_sh_redir_wait",
    // child: open file, dup2 -> stdout
    "mov eax, 2",
    "mov rdi, r10",
    "mov rsi, r11",
    "int 0x80",
    "cmp rax, -1",
    "je 9f",
    "mov r12, rax",
    "mov eax, 24",
    "mov rdi, r12",
    "mov esi, 1",
    "int 0x80",
    "mov eax, 6",
    "mov rdi, r12",
    "int 0x80",
    "mov byte ptr [r14 + r9], 0",
    "mov eax, 15",
    "mov rdi, r14",
    "mov rsi, r9",
    "int 0x80",
    "9:",
    "mov edi, 1",
    "mov eax, 3",
    "int 0x80",
    "wovenhat_sh_do_redir_in:",
    "mov r9, r8",
    "1:",
    "test r9, r9",
    "jz wovenhat_sh_redir_fail",
    "dec r9",
    "cmp byte ptr [r14 + r9], 32",
    "je 1b",
    "inc r9",
    "test r9, r9",
    "jz wovenhat_sh_redir_fail",
    "lea r10, [r14 + r8 + 1]",
    "2:",
    "cmp byte ptr [r10], 32",
    "jne 3f",
    "inc r10",
    "jmp 2b",
    "3:",
    "mov r11, r14",
    "add r11, r15",
    "sub r11, r10",
    "test r11, r11",
    "jz wovenhat_sh_redir_fail",
    "mov eax, 16",
    "int 0x80",
    "cmp rax, -1",
    "je wovenhat_sh_redir_fail",
    "test rax, rax",
    "jnz wovenhat_sh_redir_wait",
    "mov eax, 2",
    "mov rdi, r10",
    "mov rsi, r11",
    "int 0x80",
    "cmp rax, -1",
    "je 9f",
    "mov r12, rax",
    "mov eax, 24",
    "mov rdi, r12",
    "xor esi, esi",
    "int 0x80",
    "mov eax, 6",
    "mov rdi, r12",
    "int 0x80",
    "mov byte ptr [r14 + r9], 0",
    "mov eax, 15",
    "mov rdi, r14",
    "mov rsi, r9",
    "int 0x80",
    "9:",
    "mov edi, 1",
    "mov eax, 3",
    "int 0x80",
    "wovenhat_sh_redir_wait:",
    "mov r12, rax",
    "1:",
    "mov eax, 5",
    "mov rdi, r12",
    "int 0x80",
    "cmp rax, -2",
    "jne wovenhat_sh_loop",
    "mov eax, 7",
    "int 0x80",
    "jmp 1b",
    "wovenhat_sh_redir_fail:",
    "mov eax, 1",
    "mov edi, 1",
    "lea rsi, [rip + wovenhat_sh_redir_err]",
    "mov edx, 13",
    "int 0x80",
    "jmp wovenhat_sh_loop",

    "wovenhat_sh_find_bg:",
    "test r15, r15",
    "jz 2f",
    "mov rcx, r15",
    "dec rcx",
    "cmp byte ptr [r14 + rcx], 38",
    "jne 2f",
    "stc",
    "ret",
    "2:",
    "clc",
    "ret",
    "wovenhat_sh_mark_bg:",
    // strip trailing & and spaces
    "dec r15",
    "1:",
    "test r15, r15",
    "jz wovenhat_sh_after_bg",
    "mov rcx, r15",
    "dec rcx",
    "cmp byte ptr [r14 + rcx], 32",
    "jne 3f",
    "dec r15",
    "jmp 1b",
    "3:",
    "mov byte ptr [r14 + 600], 1",
    "jmp wovenhat_sh_after_bg",

    "wovenhat_sh_count_pipes:",
    "xor eax, eax",
    "xor rcx, rcx",
    "1:",
    "cmp rcx, r15",
    "jae 2f",
    "cmp byte ptr [r14 + rcx], 124",
    "jne 3f",
    "inc eax",
    "3:",
    "inc rcx",
    "jmp 1b",
    "2:",
    "ret",
    "wovenhat_sh_find_pipe:",
    "xor rcx, rcx",
    "1:",
    "cmp rcx, r15",
    "jae 2f",
    "cmp byte ptr [r14 + rcx], 124",
    "je 3f",
    "inc rcx",
    "jmp 1b",
    "2:",
    "clc",
    "ret",
    "3:",
    "mov r8, rcx",
    "stc",
    "ret",
    // left|right pipeline (single stage). Paths only (no builtins on either side).

    "wovenhat_sh_do_pipeline3:",
    // find first and second |
    "call wovenhat_sh_find_pipe",
    "jnc wovenhat_sh_pipe_fail",
    "mov r9, r8",
    // second | after first
    "mov rcx, r8",
    "inc rcx",
    "1:",
    "cmp rcx, r15",
    "jae wovenhat_sh_pipe_fail",
    "cmp byte ptr [r14 + rcx], 124",
    "je 2f",
    "inc rcx",
    "jmp 1b",
    "2:",
    "mov r10, rcx",
    // stage lengths: s0=[r14,r9), s1=(r9+1,r10), s2=(r10+1,end)
    // trim handled simply: skip spaces at starts
    "mov eax, 23",
    "int 0x80",
    "cmp rax, -1",
    "je wovenhat_sh_pipe_fail",
    "mov r12, rax",
    "and r12, 0xffffffff",
    "mov r13, rax",
    "shr r13, 32",
    "mov eax, 23",
    "int 0x80",
    "cmp rax, -1",
    "je wovenhat_sh_pipe_fail",
    "mov rbx, rax",
    "and rbx, 0xffffffff",
    "mov r11, rax",
    "shr r11, 32",
    // fork stage 0 writer to pipe0
    "mov eax, 16",
    "int 0x80",
    "cmp rax, -1",
    "je wovenhat_sh_pipe_fail",
    "test rax, rax",
    "jnz 10f",
    "mov eax, 24",
    "mov rdi, r13",
    "mov esi, 1",
    "int 0x80",
    "mov eax, 6",
    "mov rdi, r12",
    "int 0x80",
    "mov eax, 6",
    "mov rdi, r13",
    "int 0x80",
    "mov eax, 6",
    "mov rdi, rbx",
    "int 0x80",
    "mov eax, 6",
    "mov rdi, r11",
    "int 0x80",
    "mov byte ptr [r14 + r9], 0",
    "mov eax, 15",
    "mov rdi, r14",
    "mov rsi, r9",
    "int 0x80",
    "mov edi, 1",
    "mov eax, 3",
    "int 0x80",
    "10:",
    "mov qword ptr [r14 + 608], rax",
    // fork stage 1 middle
    "mov eax, 16",
    "int 0x80",
    "cmp rax, -1",
    "je wovenhat_sh_pipe_fail",
    "test rax, rax",
    "jnz 20f",
    "mov eax, 24",
    "mov rdi, r12",
    "xor esi, esi",
    "int 0x80",
    "mov eax, 24",
    "mov rdi, r11",
    "mov esi, 1",
    "int 0x80",
    "mov eax, 6",
    "mov rdi, r12",
    "int 0x80",
    "mov eax, 6",
    "mov rdi, r13",
    "int 0x80",
    "mov eax, 6",
    "mov rdi, rbx",
    "int 0x80",
    "mov eax, 6",
    "mov rdi, r11",
    "int 0x80",
    // path starts after first | 
    "lea rdi, [r14 + r9 + 1]",
    "21:",
    "cmp byte ptr [rdi], 32",
    "jne 22f",
    "inc rdi",
    "jmp 21b",
    "22:",
    "mov rsi, r10",
    "add rsi, r14",
    "sub rsi, rdi",
    "mov eax, 15",
    "int 0x80",
    "mov edi, 1",
    "mov eax, 3",
    "int 0x80",
    "20:",
    "mov qword ptr [r14 + 616], rax",
    // fork stage 2 reader
    "mov eax, 16",
    "int 0x80",
    "cmp rax, -1",
    "je wovenhat_sh_pipe_fail",
    "test rax, rax",
    "jnz 30f",
    "mov eax, 24",
    "mov rdi, rbx",
    "xor esi, esi",
    "int 0x80",
    "mov eax, 6",
    "mov rdi, r12",
    "int 0x80",
    "mov eax, 6",
    "mov rdi, r13",
    "int 0x80",
    "mov eax, 6",
    "mov rdi, rbx",
    "int 0x80",
    "mov eax, 6",
    "mov rdi, r11",
    "int 0x80",
    "lea rdi, [r14 + r10 + 1]",
    "31:",
    "cmp byte ptr [rdi], 32",
    "jne 32f",
    "inc rdi",
    "jmp 31b",
    "32:",
    "mov rsi, r14",
    "add rsi, r15",
    "sub rsi, rdi",
    "mov eax, 15",
    "int 0x80",
    "mov edi, 1",
    "mov eax, 3",
    "int 0x80",
    "30:",
    "mov r8, rax",
    // parent close all 4 ends
    "mov eax, 6",
    "mov rdi, r12",
    "int 0x80",
    "mov eax, 6",
    "mov rdi, r13",
    "int 0x80",
    "mov eax, 6",
    "mov rdi, rbx",
    "int 0x80",
    "mov eax, 6",
    "mov rdi, r11",
    "int 0x80",
    // wait three children
    "mov r12, qword ptr [r14 + 608]",
    "call wovenhat_sh_wait_one",
    "mov r12, qword ptr [r14 + 616]",
    "call wovenhat_sh_wait_one",
    "mov r12, r8",
    "call wovenhat_sh_wait_one",
    "jmp wovenhat_sh_loop",
    "wovenhat_sh_wait_one:",
    "1:",
    "mov eax, 5",
    "mov rdi, r12",
    "int 0x80",
    "cmp rax, -2",
    "jne 2f",
    "mov eax, 7",
    "int 0x80",
    "jmp 1b",
    "2:",
    "ret",
    "wovenhat_sh_do_pipeline:",
    // trim: left ends at r8, skip spaces; right starts after |
    "mov r9, r8",
    // left length in r9 (trim trailing spaces)
    "1:",
    "test r9, r9",
    "jz wovenhat_sh_pipe_fail",
    "dec r9",
    "cmp byte ptr [r14 + r9], 32",
    "je 1b",
    "inc r9",
    "test r9, r9",
    "jz wovenhat_sh_pipe_fail",
    // right start in r10
    "lea r10, [r14 + r8 + 1]",
    "2:",
    "cmp byte ptr [r10], 32",
    "jne 3f",
    "inc r10",
    "jmp 2b",
    "3:",
    // right length in r11
    "mov r11, r14",
    "add r11, r15",
    "sub r11, r10",
    "test r11, r11",
    "jz wovenhat_sh_pipe_fail",
    // pipe()
    "mov eax, 23",
    "int 0x80",
    "cmp rax, -1",
    "je wovenhat_sh_pipe_fail",
    "mov r12, rax",
    "and r12, 0xffffffff",
    "mov r13, rax",
    "shr r13, 32",
    // fork left (writer)
    "mov eax, 16",
    "int 0x80",
    "cmp rax, -1",
    "je wovenhat_sh_pipe_fail",
    "test rax, rax",
    "jnz wovenhat_sh_pipe_after_left",
    // left child: dup2(write, 1), close both, exec left
    "mov eax, 24",
    "mov rdi, r13",
    "mov esi, 1",
    "int 0x80",
    "mov eax, 6",
    "mov rdi, r12",
    "int 0x80",
    "mov eax, 6",
    "mov rdi, r13",
    "int 0x80",
    // NUL-terminate left in place
    "mov byte ptr [r14 + r9], 0",
    "mov eax, 15",
    "mov rdi, r14",
    "mov rsi, r9",
    "int 0x80",
    "mov edi, 1",
    "mov eax, 3",
    "int 0x80",
    "wovenhat_sh_pipe_after_left:",
    "mov rbx, rax",
    // fork right (reader)
    "mov eax, 16",
    "int 0x80",
    "cmp rax, -1",
    "je wovenhat_sh_pipe_fail",
    "test rax, rax",
    "jnz wovenhat_sh_pipe_parent",
    // right child: dup2(read, 0), close both, exec right
    "mov eax, 24",
    "mov rdi, r12",
    "xor esi, esi",
    "int 0x80",
    "mov eax, 6",
    "mov rdi, r12",
    "int 0x80",
    "mov eax, 6",
    "mov rdi, r13",
    "int 0x80",
    "mov eax, 15",
    "mov rdi, r10",
    "mov rsi, r11",
    "int 0x80",
    "mov edi, 1",
    "mov eax, 3",
    "int 0x80",
    "wovenhat_sh_pipe_parent:",
    "mov r8, rax",
    // parent closes both ends
    "mov eax, 6",
    "mov rdi, r12",
    "int 0x80",
    "mov eax, 6",
    "mov rdi, r13",
    "int 0x80",
    // wait left
    "mov r12, rbx",
    "wovenhat_sh_pipe_wait1:",
    "mov eax, 5",
    "mov rdi, r12",
    "int 0x80",
    "cmp rax, -2",
    "jne 1f",
    "mov eax, 7",
    "int 0x80",
    "jmp wovenhat_sh_pipe_wait1",
    "1:",
    // wait right
    "wovenhat_sh_pipe_wait2:",
    "mov eax, 5",
    "mov rdi, r8",
    "int 0x80",
    "cmp rax, -2",
    "jne wovenhat_sh_loop",
    "mov eax, 7",
    "int 0x80",
    "jmp wovenhat_sh_pipe_wait2",
    "wovenhat_sh_pipe_fail:",
    "mov eax, 1",
    "mov edi, 1",
    "lea rsi, [rip + wovenhat_sh_pipe_err]",
    "mov edx, 12",
    "int 0x80",
    "jmp wovenhat_sh_loop",
    "wovenhat_sh_do_exec:",
    // fork
    "mov eax, 16",
    "int 0x80",
    "cmp rax, -1",
    "je wovenhat_sh_exec_fail",
    "test rax, rax",
    "jz wovenhat_sh_child",
    // parent: if background, print pid and continue
    "cmp byte ptr [r14 + 600], 1",
    "jne wovenhat_sh_fg_wait",
    "mov r12, rax",
    "mov rax, r12",
    "call wovenhat_sh_print_u",
    "mov eax, 1",
    "mov edi, 1",
    "lea rsi, [rip + wovenhat_sh_nl]",
    "mov edx, 1",
    "int 0x80",
    "jmp wovenhat_sh_loop",
    "wovenhat_sh_fg_wait:",
    "mov r12, rax",
    "wovenhat_sh_wait:",
    "mov eax, 5",
    "mov rdi, r12",
    "int 0x80",
    "cmp rax, -2",
    "jne wovenhat_sh_loop",
    "mov eax, 7",
    "int 0x80",
    "jmp wovenhat_sh_wait",
    "wovenhat_sh_child:",
    // exec(path=r14, len=r15)
    "mov eax, 15",
    "mov rdi, r14",
    "mov rsi, r15",
    "int 0x80",
    // exec failed
    "mov edi, 1",
    "mov eax, 3",
    "int 0x80",
    "wovenhat_sh_exec_fail:",
    "mov eax, 1",
    "mov edi, 1",
    "lea rsi, [rip + wovenhat_sh_exec_err]",
    "mov edx, 12",
    "int 0x80",
    "jmp wovenhat_sh_loop",
    "wovenhat_sh_die:",
    "mov edi, 1",
    "mov eax, 3",
    "int 0x80",
    // strcmp-ish: ZF path — set CF if [r14] equals C string in rdi (exact)
    "wovenhat_sh_eq:",
    "xor rcx, rcx",
    "1:",
    "mov al, [rdi + rcx]",
    "mov bl, [r14 + rcx]",
    "cmp al, 0",
    "je 2f",
    "cmp al, bl",
    "jne 3f",
    "inc rcx",
    "jmp 1b",
    "2:",
    "cmp bl, 0",
    "jne 3f",
    "stc",
    "ret",
    "3:",
    "clc",
    "ret",
    // prefix: CF if line starts with C string in rdi followed by NUL or space
    "wovenhat_sh_prefix:",
    "xor rcx, rcx",
    "1:",
    "mov al, [rdi + rcx]",
    "cmp al, 0",
    "je 2f",
    "cmp al, [r14 + rcx]",
    "jne 3f",
    "inc rcx",
    "jmp 1b",
    "2:",
    "mov bl, [r14 + rcx]",
    "cmp bl, 0",
    "je 4f",
    "cmp bl, 32",
    "jne 3f",
    "4:",
    "stc",
    "ret",
    "3:",
    "clc",
    "ret",
    "wovenhat_sh_banner:",
    ".ascii \"WovenHat userspace shell\\n\\n\"",
    "wovenhat_sh_prompt:",
    ".ascii \"sh> \"",
    "wovenhat_sh_nl:",
    ".ascii \"\\n\"",
    "wovenhat_sh_bksp_seq:",
    ".ascii \"\\x08 \\x08\"",
    "wovenhat_sh_slash:",
    ".ascii \"/\"",
    "wovenhat_sh_cmd_exit:",
    ".asciz \"exit\"",
    "wovenhat_sh_cmd_quit:",
    ".asciz \"quit\"",
    "wovenhat_sh_cmd_help:",
    ".asciz \"help\"",
    "wovenhat_sh_cmd_echo:",
    ".asciz \"echo\"",
    "wovenhat_sh_cmd_pwd:",
    ".asciz \"pwd\"",
    "wovenhat_sh_cmd_cd:",
    ".asciz \"cd\"",
    "wovenhat_sh_cmd_cat:",
    ".asciz \"cat\"",
    "wovenhat_sh_cmd_ls:",
    ".asciz \"ls\"",
    "wovenhat_sh_cmd_mkdir:",
    ".asciz \"mkdir\"",
    "wovenhat_sh_cmd_kill:",
    ".asciz \"kill\"",
    "wovenhat_sh_cmd_rm:",
    ".asciz \"rm\"",
    "wovenhat_sh_cmd_sleep:",
    ".asciz \"sleep\"",
    "wovenhat_sh_cmd_getpid:",
    ".asciz \"getpid\"",
    "wovenhat_sh_cmd_mv:",
    ".asciz \"mv\"",
    "wovenhat_sh_cmd_getppid:",
    ".asciz \"getppid\"",
    "wovenhat_sh_cmd_ticks:",
    ".asciz \"ticks\"",
    "wovenhat_sh_cmd_clear:",
    ".asciz \"clear\"",
    "wovenhat_sh_cmd_yield:",
    ".asciz \"yield\"",
    "wovenhat_sh_cmd_which:",
    ".asciz \"which\"",
    "wovenhat_sh_cmd_wait:",
    ".asciz \"wait\"",
    "wovenhat_sh_cmd_test:",
    ".asciz \"test\"",
    "wovenhat_sh_cmd_colon:",
    ".asciz \":\"",
    "wovenhat_sh_help:",
    ".ascii \"cmds: help echo cat ls mkdir cd pwd kill exit; | > <\\n\"",
    "wovenhat_sh_cd_fail:",
    ".ascii \"cd: fail\\n\"",
    "wovenhat_sh_exec_err:",
    ".ascii \"exec failed\\n\"",
    "wovenhat_sh_cat_usage_msg:",
    ".ascii \"usage: cat p\\n\"",
    "wovenhat_sh_cat_fail_msg:",
    ".ascii \"cat: fail\\n\"",
    "wovenhat_sh_ls_d:",
    ".ascii \"d \"",
    "wovenhat_sh_ls_f:",
    ".ascii \"f \"",
    "wovenhat_sh_ls_err:",
    ".ascii \"ls: fail\\n\"",
    "wovenhat_sh_mkdir_usage_msg:",
    ".ascii \"usage: mkdir p\\n\"",
    "wovenhat_sh_mkdir_fail_msg:",
    ".ascii \"mkdir: fail\\n\"",
    "wovenhat_sh_pipe_err:",
    ".ascii \"pipe failed\\n\"",
    "wovenhat_sh_kill_usage_msg:",
    ".ascii \"usage: kill n\\n\"",
    "wovenhat_sh_kill_fail_msg:",
    ".ascii \"kill: fail\\n\"",
    "wovenhat_sh_redir_err:",
    ".ascii \"redir failed\\n\"",
    "wovenhat_sh_rm_usage_msg:",
    ".ascii \"usage: rm p\\n\"",
    "wovenhat_sh_rm_fail_msg:",
    ".ascii \"rm: fail\\n\"",
    "wovenhat_sh_sleep_usage_msg:",
    ".ascii \"usage: sleep n\\n\"",
    "wovenhat_sh_mv_usage_msg:",
    ".ascii \"usage: mv a b\\n\"",
    "wovenhat_sh_mv_fail_msg:",
    ".ascii \"mv: fail\\n\"",
    "wovenhat_sh_clear_seq:",
    ".byte 0x1b, 0x5b, 0x32, 0x4a, 0x1b, 0x5b, 0x48",
    "wovenhat_sh_which_usage_msg:",
    ".ascii \"usage: which n\\n\"",
    "wovenhat_sh_which_fail_msg:",
    ".ascii \"not found\\n\"",
    "wovenhat_sh_wait_usage_msg:",
    ".ascii \"usage: wait n\\n\"",
    "wovenhat_sh_test_fail_msg:",
    ".ascii \"false\\n\"",
    "wovenhat_sh_program_end:",
    ".previous",
);

/// Minimal freestanding "libc" — C-callable syscall stubs for userspace.
///
/// Calling convention: System V AMD64 (args in rdi, rsi, rdx…).
/// These symbols are embedded for documentation and future static linking;
/// the interactive shell uses the same syscall numbers directly.
global_asm!(
    ".section .rodata.wovenhat_libc, \"a\"",
    ".global wovenhat_libc_start",
    ".global wovenhat_libc_end",
    "wovenhat_libc_start:",
    ".global wovenhat_sys_read",
    "wovenhat_sys_read:",
    "mov eax, 0",
    "int 0x80",
    "ret",
    ".global wovenhat_sys_write",
    "wovenhat_sys_write:",
    "mov eax, 1",
    "int 0x80",
    "ret",
    ".global wovenhat_sys_open",
    "wovenhat_sys_open:",
    "mov eax, 2",
    "int 0x80",
    "ret",
    ".global wovenhat_sys_exit",
    "wovenhat_sys_exit:",
    "mov eax, 3",
    "int 0x80",
    "ret",
    ".global wovenhat_sys_getpid",
    "wovenhat_sys_getpid:",
    "mov eax, 4",
    "int 0x80",
    "ret",
    ".global wovenhat_sys_waitpid",
    "wovenhat_sys_waitpid:",
    "mov eax, 5",
    "int 0x80",
    "ret",
    ".global wovenhat_sys_close",
    "wovenhat_sys_close:",
    "mov eax, 6",
    "int 0x80",
    "ret",
    ".global wovenhat_sys_yield",
    "wovenhat_sys_yield:",
    "mov eax, 7",
    "int 0x80",
    "ret",
    ".global wovenhat_sys_fork",
    "wovenhat_sys_fork:",
    "mov eax, 16",
    "int 0x80",
    "ret",
    ".global wovenhat_sys_exec",
    "wovenhat_sys_exec:",
    "mov eax, 15",
    "int 0x80",
    "ret",
    ".global wovenhat_sys_pipe",
    "wovenhat_sys_pipe:",
    // C ABI: int pipe(int fds[2]); we return packed and let caller split,
    // or: rdi = pointer to int[2]
    "push rdi",
    "mov eax, 23",
    "int 0x80",
    "pop rdi",
    "cmp rax, -1",
    "je 1f",
    "mov ecx, eax",
    "mov dword ptr [rdi], ecx",
    "shr rax, 32",
    "mov dword ptr [rdi + 4], eax",
    "xor eax, eax",
    "ret",
    "1:",
    "mov eax, -1",
    "ret",
    ".global wovenhat_sys_dup2",
    "wovenhat_sys_dup2:",
    "mov eax, 24",
    "int 0x80",
    "ret",
    ".global wovenhat_sys_getppid",
    "wovenhat_sys_getppid:",
    "mov eax, 25",
    "int 0x80",
    "ret",
    ".global wovenhat_sys_kill",
    "wovenhat_sys_kill:",
    "mov eax, 26",
    "int 0x80",
    "ret",
    ".global wovenhat_sys_lseek",
    "wovenhat_sys_lseek:",
    "mov eax, 27",
    "int 0x80",
    "ret",
    ".global wovenhat_sys_unlink",
    "wovenhat_sys_unlink:",
    "mov eax, 28",
    "int 0x80",
    "ret",
    ".global wovenhat_sys_sleep",
    "wovenhat_sys_sleep:",
    "mov eax, 29",
    "int 0x80",
    "ret",
    ".global wovenhat_sys_rename",
    "wovenhat_sys_rename:",
    "mov eax, 30",
    "int 0x80",
    "ret",
    ".global wovenhat_sys_getticks",
    "wovenhat_sys_getticks:",
    "mov eax, 31",
    "int 0x80",
    "ret",
    ".global wovenhat_sys_sync",
    "wovenhat_sys_sync:",
    "mov eax, 32",
    "int 0x80",
    "ret",
    // --- minimal heap (bump allocator over one mmap region) ---
    // layout of heap page: [0]=base, [8]=curr, [16]=end  (all u64)
    ".global wovenhat_heap_init",
    "wovenhat_heap_init:",
    // mmap 64KiB writable
    "mov eax, 8",
    "mov edi, 65536",
    "mov esi, 1",
    "int 0x80",
    "cmp rax, -1",
    "je 1f",
    "mov r12, rax",
    "lea rax, [r12 + 24]",
    "mov qword ptr [r12], rax",
    "mov qword ptr [r12 + 8], rax",
    "lea rax, [r12 + 65536]",
    "mov qword ptr [r12 + 16], rax",
    "mov qword ptr [r12 + 24], 0",
    "mov rax, r12",
    "ret",
    "1:",
    "xor eax, eax",
    "ret",
    ".global wovenhat_malloc",
    "wovenhat_malloc:",
    // rdi = size, rsi = heap handle from heap_init
    "test rsi, rsi",
    "jz 2f",
    "mov rax, [rsi + 8]",
    "add rdi, 7",
    "and rdi, 0xfffffffffffffff8",
    "mov rcx, rax",
    "add rcx, rdi",
    "cmp rcx, [rsi + 16]",
    "ja 2f",
    "mov [rsi + 8], rcx",
    "ret",
    "2:",
    "xor eax, eax",
    "ret",
    ".global wovenhat_puts",
    "wovenhat_puts:",
    "mov rsi, rdi",
    "xor edx, edx",
    "1:",
    "cmp byte ptr [rsi + rdx], 0",
    "je 2f",
    "inc rdx",
    "jmp 1b",
    "2:",
    "mov eax, 1",
    "mov edi, 1",
    "int 0x80",
    "ret",
    ".global wovenhat_free",
    "wovenhat_free:",
    // freelist: [ptr]=next, store size at [ptr+8]; rsi=heap handle, rdi=ptr
    "test rdi, rdi",
    "jz 1f",
    "test rsi, rsi",
    "jz 1f",
    "mov rax, [rsi + 24]",
    "mov [rdi], rax",
    "mov qword ptr [rdi + 8], 0",
    "mov [rsi + 24], rdi",
    "1:",
    "xor eax, eax",
    "ret",
    ".global wovenhat_strlen",
    "wovenhat_strlen:",
    "xor eax, eax",
    "1:",
    "cmp byte ptr [rdi + rax], 0",
    "je 2f",
    "inc rax",
    "jmp 1b",
    "2:",
    "ret",
    // wovenhat_printf(fmt, ...): supports %s %d %c %%  (max 3 args in rsi,rdx,rcx)
    ".global wovenhat_printf",
    "wovenhat_printf:",
    "push rbx",
    "push r12",
    "push r13",
    "push r14",
    "push r15",
    "mov r12, rdi",
    "mov r13, rsi",
    "mov r14, rdx",
    "mov r15, rcx",
    "xor ebx, ebx",
    "wovenhat_printf_loop:",
    "movzx eax, byte ptr [r12]",
    "test al, al",
    "jz wovenhat_printf_done",
    "cmp al, 37",
    "je wovenhat_printf_pct",
    "lea rsi, [r12]",
    "mov edx, 1",
    "mov eax, 1",
    "mov edi, 1",
    "int 0x80",
    "inc r12",
    "jmp wovenhat_printf_loop",
    "wovenhat_printf_pct:",
    "inc r12",
    "movzx eax, byte ptr [r12]",
    "cmp al, 37",
    "je wovenhat_printf_pctpct",
    "cmp al, 115",
    "je wovenhat_printf_s",
    "cmp al, 100",
    "je wovenhat_printf_d",
    "cmp al, 99",
    "je wovenhat_printf_c",
    "cmp al, 120",
    "je wovenhat_printf_x",
    "inc r12",
    "jmp wovenhat_printf_loop",
    "wovenhat_printf_pctpct:",
    "lea rsi, [rip + wovenhat_printf_pctch]",
    "mov edx, 1",
    "mov eax, 1",
    "mov edi, 1",
    "int 0x80",
    "inc r12",
    "jmp wovenhat_printf_loop",
    "wovenhat_printf_s:",
    "inc r12",
    "cmp ebx, 0",
    "jne 1f",
    "mov rdi, r13",
    "inc ebx",
    "jmp 2f",
    "1:",
    "cmp ebx, 1",
    "jne 3f",
    "mov rdi, r14",
    "inc ebx",
    "jmp 2f",
    "3:",
    "mov rdi, r15",
    "inc ebx",
    "2:",
    "call wovenhat_puts",
    "jmp wovenhat_printf_loop",
    "wovenhat_printf_c:",
    "inc r12",
    "cmp ebx, 0",
    "jne 1f",
    "mov rax, r13",
    "inc ebx",
    "jmp 2f",
    "1:",
    "cmp ebx, 1",
    "jne 3f",
    "mov rax, r14",
    "inc ebx",
    "jmp 2f",
    "3:",
    "mov rax, r15",
    "inc ebx",
    "2:",
    "mov byte ptr [rip + wovenhat_printf_ch], al",
    "lea rsi, [rip + wovenhat_printf_ch]",
    "mov edx, 1",
    "mov eax, 1",
    "mov edi, 1",
    "int 0x80",
    "jmp wovenhat_printf_loop",
    "wovenhat_printf_d:",
    "inc r12",
    "cmp ebx, 0",
    "jne 1f",
    "mov rax, r13",
    "inc ebx",
    "jmp 2f",
    "1:",
    "cmp ebx, 1",
    "jne 3f",
    "mov rax, r14",
    "inc ebx",
    "jmp 2f",
    "3:",
    "mov rax, r15",
    "inc ebx",
    "2:",
    "call wovenhat_print_int",
    "jmp wovenhat_printf_loop",
    "wovenhat_printf_x:",
    "inc r12",
    "cmp ebx, 0",
    "jne 1f",
    "mov rax, r13",
    "inc ebx",
    "jmp 2f",
    "1:",
    "cmp ebx, 1",
    "jne 3f",
    "mov rax, r14",
    "inc ebx",
    "jmp 2f",
    "3:",
    "mov rax, r15",
    "inc ebx",
    "2:",
    "call wovenhat_print_hex",
    "jmp wovenhat_printf_loop",
    "wovenhat_print_hex:",
    "push rbx",
    "push rcx",
    "push rdx",
    "lea rbx, [rip + wovenhat_printf_numbuf + 20]",
    "mov byte ptr [rbx], 0",
    "mov rcx, rax",
    "mov rdi, 16",
    "1:",
    "mov rax, rcx",
    "xor edx, edx",
    "div rdi",
    "mov rcx, rax",
    "cmp dl, 10",
    "jb 2f",
    "add dl, 87",
    "jmp 3f",
    "2:",
    "add dl, 48",
    "3:",
    "dec rbx",
    "mov [rbx], dl",
    "test rcx, rcx",
    "jnz 1b",
    "mov rsi, rbx",
    "lea rdx, [rip + wovenhat_printf_numbuf + 20]",
    "sub rdx, rsi",
    "mov eax, 1",
    "mov edi, 1",
    "int 0x80",
    "pop rdx",
    "pop rcx",
    "pop rbx",
    "ret",
    "wovenhat_printf_done:",
    "pop r15",
    "pop r14",
    "pop r13",
    "pop r12",
    "pop rbx",
    "xor eax, eax",
    "ret",
    "wovenhat_print_int:",
    // print signed 64-bit in rax
    "push rbx",
    "push rcx",
    "push rdx",
    "mov rcx, rax",
    "test rcx, rcx",
    "jns 1f",
    "neg rcx",
    "lea rsi, [rip + wovenhat_printf_minus]",
    "mov edx, 1",
    "mov eax, 1",
    "mov edi, 1",
    "int 0x80",
    "1:",
    "lea rbx, [rip + wovenhat_printf_numbuf + 20]",
    "mov byte ptr [rbx], 0",
    "mov rax, rcx",
    "mov rdi, 10",
    "2:",
    "xor edx, edx",
    "div rdi",
    "add dl, 48",
    "dec rbx",
    "mov [rbx], dl",
    "test rax, rax",
    "jnz 2b",
    "mov rsi, rbx",
    "lea rdx, [rip + wovenhat_printf_numbuf + 20]",
    "sub rdx, rsi",
    "mov eax, 1",
    "mov edi, 1",
    "int 0x80",
    "pop rdx",
    "pop rcx",
    "pop rbx",
    "ret",
    "wovenhat_printf_pctch:",
    ".byte 37",
    "wovenhat_printf_minus:",
    ".byte 45",
    "wovenhat_printf_ch:",
    ".byte 0",
    "wovenhat_printf_numbuf:",
    ".space 24",
    "wovenhat_libc_end:",
    ".previous",
);


// /bin/echo — print argv[1..] separated by spaces
global_asm!(
    ".section .rodata.wovenhat_echo_stub, \"a\"",
    ".global wovenhat_echo_program_start",
    ".global wovenhat_echo_program_end",
    "wovenhat_echo_program_start:",
    "mov r12, [rsp]",
    "lea r13, [rsp + 8]",
    "cmp r12, 2",
    "jb wovenhat_echo_nl",
    "mov r14, 1",
    "wovenhat_echo_arg:",
    "cmp r14, r12",
    "jae wovenhat_echo_nl",
    "mov rsi, [r13 + r14 * 8]",
    "xor edx, edx",
    "1:",
    "cmp byte ptr [rsi + rdx], 0",
    "je 2f",
    "inc rdx",
    "jmp 1b",
    "2:",
    "mov eax, 1",
    "mov edi, 1",
    "int 0x80",
    "inc r14",
    "cmp r14, r12",
    "jae wovenhat_echo_nl",
    "mov eax, 1",
    "mov edi, 1",
    "lea rsi, [rip + wovenhat_echo_sp]",
    "mov edx, 1",
    "int 0x80",
    "jmp wovenhat_echo_arg",
    "wovenhat_echo_nl:",
    "mov eax, 1",
    "mov edi, 1",
    "lea rsi, [rip + wovenhat_echo_nlc]",
    "mov edx, 1",
    "int 0x80",
    "xor edi, edi",
    "mov eax, 3",
    "int 0x80",
    "wovenhat_echo_sp:",
    ".ascii \" \"",
    "wovenhat_echo_nlc:",
    ".ascii \"\\n\"",
    "wovenhat_echo_program_end:",
    ".previous",
);


global_asm!(
    ".section .rodata.wovenhat_true_stub, \"a\"",
    ".global wovenhat_true_program_start",
    ".global wovenhat_true_program_end",
    "wovenhat_true_program_start:",
    "xor edi, edi",
    "mov eax, 3",
    "int 0x80",
    "wovenhat_true_program_end:",
    ".previous",
);

global_asm!(
    ".section .rodata.wovenhat_false_stub, \"a\"",
    ".global wovenhat_false_program_start",
    ".global wovenhat_false_program_end",
    "wovenhat_false_program_start:",
    "mov edi, 1",
    "mov eax, 3",
    "int 0x80",
    "wovenhat_false_program_end:",
    ".previous",
);

// /bin/cat — open argv[1], copy to stdout
global_asm!(
    ".section .rodata.wovenhat_cat_stub, \"a\"",
    ".global wovenhat_cat_program_start",
    ".global wovenhat_cat_program_end",
    "wovenhat_cat_program_start:",
    "mov r12, [rsp]",
    "cmp r12, 2",
    "jb wovenhat_cat_usage",
    "mov rdi, [rsp + 16]",
    // strlen
    "xor esi, esi",
    "1:",
    "cmp byte ptr [rdi + rsi], 0",
    "je 2f",
    "inc rsi",
    "jmp 1b",
    "2:",
    "mov eax, 2",
    "int 0x80",
    "cmp rax, -1",
    "je wovenhat_cat_fail",
    "mov r13, rax",
    "sub rsp, 128",
    "wovenhat_cat_loop:",
    "mov eax, 0",
    "mov rdi, r13",
    "mov rsi, rsp",
    "mov edx, 128",
    "int 0x80",
    "cmp rax, -1",
    "je wovenhat_cat_done",
    "test rax, rax",
    "jz wovenhat_cat_done",
    "mov rdx, rax",
    "mov eax, 1",
    "mov edi, 1",
    "mov rsi, rsp",
    "int 0x80",
    "jmp wovenhat_cat_loop",
    "wovenhat_cat_done:",
    "mov eax, 6",
    "mov rdi, r13",
    "int 0x80",
    "xor edi, edi",
    "mov eax, 3",
    "int 0x80",
    "wovenhat_cat_usage:",
    "wovenhat_cat_fail:",
    "mov edi, 1",
    "mov eax, 3",
    "int 0x80",
    "wovenhat_cat_program_end:",
    ".previous",
);


// /bin/ls — list directory (argv[1] or ".")
global_asm!(
    ".section .rodata.wovenhat_ls_stub, \"a\"",
    ".global wovenhat_ls_program_start",
    ".global wovenhat_ls_program_end",
    "wovenhat_ls_program_start:",
    "mov r12, [rsp]",
    "cmp r12, 2",
    "jb wovenhat_ls_dot",
    "mov r13, [rsp + 16]",
    "xor r14, r14",
    "1:",
    "cmp byte ptr [r13 + r14], 0",
    "je 2f",
    "inc r14",
    "jmp 1b",
    "2:",
    "jmp wovenhat_ls_go",
    "wovenhat_ls_dot:",
    // getcwd into buffer
    "lea r13, [rip + wovenhat_ls_cwd]",
    "mov eax, 21",
    "mov rdi, r13",
    "mov esi, 120",
    "int 0x80",
    "cmp rax, -1",
    "je wovenhat_ls_done",
    "mov r14, rax",
    "wovenhat_ls_go:",
    "xor r15, r15",
    "wovenhat_ls_loop:",
    "mov rsi, r14",
    "mov rax, r15",
    "shl rax, 16",
    "or rsi, rax",
    "mov eax, 18",
    "mov rdi, r13",
    "lea rdx, [rip + wovenhat_ls_name]",
    "int 0x80",
    "cmp rax, -1",
    "je wovenhat_ls_done",
    "mov rbx, rax",
    "and edx, 0xff",
    "mov rdx, rbx",
    "and edx, 0xff",
    "mov eax, 1",
    "mov edi, 1",
    "lea rsi, [rip + wovenhat_ls_name]",
    "int 0x80",
    "mov eax, 1",
    "mov edi, 1",
    "lea rsi, [rip + wovenhat_ls_nl]",
    "mov edx, 1",
    "int 0x80",
    "inc r15",
    "cmp r15, 64",
    "jb wovenhat_ls_loop",
    "wovenhat_ls_done:",
    "xor edi, edi",
    "mov eax, 3",
    "int 0x80",
    "wovenhat_ls_cwd:",
    ".space 128",
    "wovenhat_ls_nl:",
    ".ascii \"\\n\"",
    "wovenhat_ls_name:",
    ".space 64",
    "wovenhat_ls_program_end:",
    ".previous",
);

// /bin/sleep — sleep argv[1] ticks
global_asm!(
    ".section .rodata.wovenhat_sleepbin_stub, \"a\"",
    ".global wovenhat_sleepbin_program_start",
    ".global wovenhat_sleepbin_program_end",
    "wovenhat_sleepbin_program_start:",
    "mov r12, [rsp]",
    "cmp r12, 2",
    "jb wovenhat_sleepbin_def",
    "mov rsi, [rsp + 16]",
    "xor eax, eax",
    "xor ecx, ecx",
    "1:",
    "movzx edx, byte ptr [rsi + rcx]",
    "cmp dl, 0",
    "je 2f",
    "sub dl, 48",
    "cmp dl, 9",
    "ja 2f",
    "imul eax, eax, 10",
    "add eax, edx",
    "inc rcx",
    "jmp 1b",
    "2:",
    "mov edi, eax",
    "jmp wovenhat_sleepbin_go",
    "wovenhat_sleepbin_def:",
    "mov edi, 10",
    "wovenhat_sleepbin_go:",
    "mov eax, 29",
    "int 0x80",
    "xor edi, edi",
    "mov eax, 3",
    "int 0x80",
    "wovenhat_sleepbin_program_end:",
    ".previous",
);

unsafe extern "C" {
    static wovenhat_user_program_start: u8;
    static wovenhat_user_program_end: u8;
    static wovenhat_exec_program_start: u8;
    static wovenhat_exec_program_end: u8;
    static wovenhat_init_program_start: u8;
    static wovenhat_init_program_end: u8;
    static wovenhat_sh_program_start: u8;
    static wovenhat_sh_program_end: u8;
    static wovenhat_echo_program_start: u8;
    static wovenhat_echo_program_end: u8;
    static wovenhat_true_program_start: u8;
    static wovenhat_true_program_end: u8;
    static wovenhat_false_program_start: u8;
    static wovenhat_false_program_end: u8;
    static wovenhat_cat_program_start: u8;
    static wovenhat_cat_program_end: u8;
    static wovenhat_ls_program_start: u8;
    static wovenhat_ls_program_end: u8;
    static wovenhat_sleepbin_program_start: u8;
    static wovenhat_sleepbin_program_end: u8;
}
#[derive(Clone, Copy, Debug)]
pub struct UserImage {
    pub entry: u64,
    pub stack_top: u64,
    pub image_size: u64,
    pub load_segments: usize,
}

impl UserImage {
    pub fn is_valid(self) -> bool {
        self.entry >= USER_REGION_START
            && self.stack_top != 0
            && self.stack_top.is_multiple_of(16)
            && self.image_size != 0
            && self.load_segments != 0
    }
}

#[derive(Clone, Copy, Debug)]
pub struct UserStack {
    pub guard_base: u64,
    pub base: u64,
    pub top: u64,
    pub size: usize,
}

impl UserStack {
    pub const GUARD_SIZE: usize = 4096;
    pub const SIZE: usize = 4096 * 2;

    pub fn new(base: u64) -> Self {
        Self {
            guard_base: base - Self::GUARD_SIZE as u64,
            base,
            top: base + Self::SIZE as u64,
            size: Self::SIZE,
        }
    }

    pub fn is_aligned(self) -> bool {
        self.guard_base.is_multiple_of(4096)
            && self.base == self.guard_base + Self::GUARD_SIZE as u64
            && self.base.is_multiple_of(16)
            && self.top.is_multiple_of(16)
    }
}

/// Maximum number of argv strings placed on the initial user stack.
pub const MAX_ARGV: usize = 8;
/// Maximum total bytes for all argv string data (including NULs).
pub const MAX_ARGV_BYTES: usize = 512;

/// Minimal crt0 contract expected by the kernel stack layout:
///
/// ```asm
/// _start:
///     mov rdi, [rsp]       ; argc
///     lea rsi, [rsp+8]     ; argv
///     call main
///     mov eax, 3           ; exit
///     mov rdi, rax
///     int 0x80
/// ```
///
/// Build a System V-style initial stack for a new user process.
///
/// Layout (high → low addresses):
/// ```text
///   [ string area: argv[0]\0 argv[1]\0 ... ]
///   [ padding to 16-byte alignment ]
///   NULL          (envp terminator — empty environment for now)
///   NULL          (argv terminator)
///   argv[n-1]
///   ...
///   argv[0]
///   argc
///   ← rsp (16-byte aligned)
/// ```
///
/// On entry the process may read argc from `[rsp]` and argv from `[rsp+8]`.
/// `envp` starts after the argv NULL. Registers are not set.
///
/// Returns the new stack pointer (value to place in RSP), or `None` on failure.
pub fn setup_argv_stack(
    address_space: paging::AddressSpace,
    stack: UserStack,
    args: &[&str],
) -> Option<u64> {
    if args.len() > MAX_ARGV {
        return None;
    }
    let mut string_bytes = 0usize;
    for arg in args {
        string_bytes = string_bytes.checked_add(arg.len().checked_add(1)?)?;
    }
    if string_bytes > MAX_ARGV_BYTES {
        return None;
    }

    let strings_start = stack.top.checked_sub(string_bytes as u64)?;
    if strings_start < stack.base {
        return None;
    }

    let mut cursor = strings_start;
    let mut argv_ptrs = [0u64; MAX_ARGV];
    for (i, arg) in args.iter().enumerate() {
        argv_ptrs[i] = cursor;
        let mut buf = [0u8; 256];
        if arg.len() >= buf.len() {
            return None;
        }
        buf[..arg.len()].copy_from_slice(arg.as_bytes());
        paging::write_user_bytes(address_space, cursor, &buf[..arg.len() + 1]).ok()?;
        cursor = cursor.checked_add(arg.len() as u64 + 1)?;
    }

    // argc + argv... + NULL + envp NULL
    let argc = args.len() as u64;
    let pointer_words = 1 + args.len() + 1 + 1;
    let pointer_bytes = pointer_words * 8;
    let mut rsp = strings_start.checked_sub(pointer_bytes as u64)?;
    rsp &= !0xfu64;
    if rsp < stack.base {
        return None;
    }

    paging::write_user_bytes(address_space, rsp, &argc.to_le_bytes()).ok()?;
    for i in 0..args.len() {
        let addr = rsp + 8 + (i as u64) * 8;
        paging::write_user_bytes(address_space, addr, &argv_ptrs[i].to_le_bytes()).ok()?;
    }
    let argv_null = rsp + 8 + (args.len() as u64) * 8;
    paging::write_user_bytes(address_space, argv_null, &0u64.to_le_bytes()).ok()?;
    let envp_null = argv_null + 8;
    paging::write_user_bytes(address_space, envp_null, &0u64.to_le_bytes()).ok()?;

    Some(rsp)
}

const USER_MMAP_START: u64 = USER_REGION_START + 0x10_0000;
const USER_MMAP_STRIDE: u64 = 0x10_000;
const USER_MMAP_MAX_SIZE: usize = USER_MMAP_STRIDE as usize;

#[derive(Clone, Copy)]
pub struct AnonymousMapping {
    pub address: u64,
    pub size: usize,
    pub writable: bool,
}
#[derive(Clone, Copy)]
struct UserMapping {
    start: u64,
    size: usize,
    writable: bool,
    executable: bool,
}

impl UserMapping {
    const EMPTY: Self = Self {
        start: 0,
        size: 0,
        writable: false,
        executable: false,
    };
}

#[derive(Clone, Copy)]
pub struct AddressSpace {
    paging: paging::AddressSpace,
    stack_base: u64,
    mappings: [UserMapping; MAX_ELF_SEGMENTS],
    mapping_count: usize,
}

impl AddressSpace {
    pub fn paging(self) -> paging::AddressSpace {
        self.paging
    }

    pub fn root_address(self) -> u64 {
        self.paging.root_address()
    }

    /// Returns whether `address` falls in a logically writable mapping
    /// (ELF segment, user stack, or anonymous region tracked by the process).
    pub fn is_logically_writable(
        self,
        address: u64,
        anonymous: &[Option<AnonymousMapping>; MAX_ANONYMOUS_MAPPINGS],
    ) -> bool {
        if address >= self.stack_base && address < self.stack_base + UserStack::SIZE as u64 {
            return true;
        }
        for mapping in &self.mappings[..self.mapping_count] {
            if mapping.writable
                && address >= mapping.start
                && address < mapping.start + mapping.size as u64
            {
                return true;
            }
        }
        for mapping in anonymous.iter().flatten() {
            if mapping.writable
                && address >= mapping.address
                && address < mapping.address + mapping.size as u64
            {
                return true;
            }
        }
        false
    }
}

#[derive(Clone, Copy)]
pub struct UserProgram {
    pub image: UserImage,
    pub stack: UserStack,
    pub address_space: AddressSpace,
}

pub fn elf_loader_self_test() -> bool {
    let stub = unsafe {
        let start = &wovenhat_user_program_start as *const u8;
        let end = &wovenhat_user_program_end as *const u8;
        core::slice::from_raw_parts(start, end.offset_from(start) as usize)
    };
    let Some(valid) = build_stub_elf(stub) else {
        return false;
    };
    let Ok(image) = crate::elf::parse(&valid) else {
        return false;
    };
    let valid_segment = image.segment_count() == 1
        && image.entry == USER_REGION_START
        && image
            .segments()
            .next()
            .is_some_and(|segment| segment.memory_size == USER_CODE_SIZE && segment.executable);

    let mut bad_magic = valid.clone();
    bad_magic[0] = 0;
    let mut writable_executable = valid;
    writable_executable[68] = 7;
    valid_segment
        && crate::elf::parse(&bad_magic).is_err()
        && crate::elf::parse(&writable_executable).is_err()
}
pub fn create_stub_process() -> Option<UserProgram> {
    let stub = unsafe {
        let start = &wovenhat_user_program_start as *const u8;
        let end = &wovenhat_user_program_end as *const u8;
        core::slice::from_raw_parts(start, end.offset_from(start) as usize)
    };
    let elf = build_stub_elf(stub)?;
    load_elf(&elf)
}

pub fn install_stub_executable() -> bool {
    let stub = unsafe {
        let start = &wovenhat_user_program_start as *const u8;
        let end = &wovenhat_user_program_end as *const u8;
        core::slice::from_raw_parts(start, end.offset_from(start) as usize)
    };
    build_stub_elf(stub)
        .is_some_and(|elf| crate::vfs::create_read_only("/bin/selftest", &elf).is_ok())
}

pub fn create_exec_process() -> Option<UserProgram> {
    let stub = unsafe {
        let start = &wovenhat_exec_program_start as *const u8;
        let end = &wovenhat_exec_program_end as *const u8;
        core::slice::from_raw_parts(start, end.offset_from(start) as usize)
    };
    let elf = build_stub_elf(stub)?;
    load_elf(&elf)
}

/// Spawnable userspace init process (`argv[0] == "/init"`).
pub fn create_init_process() -> Option<UserProgram> {
    let stub = unsafe {
        let start = &wovenhat_init_program_start as *const u8;
        let end = &wovenhat_init_program_end as *const u8;
        core::slice::from_raw_parts(start, end.offset_from(start) as usize)
    };
    let elf = build_stub_elf(stub)?;
    load_elf_with_argv(&elf, &["/init"])
}

/// Install `/bin/init` into the VFS for later `exec`.
pub fn install_init_executable() -> bool {
    let stub = unsafe {
        let start = &wovenhat_init_program_start as *const u8;
        let end = &wovenhat_init_program_end as *const u8;
        core::slice::from_raw_parts(start, end.offset_from(start) as usize)
    };
    build_stub_elf(stub).is_some_and(|elf| crate::vfs::create_read_only("/bin/init", &elf).is_ok())
}

/// Interactive userspace shell (`argv[0] == "/bin/sh"`).
pub fn create_shell_process() -> Option<UserProgram> {
    let stub = unsafe {
        let start = &wovenhat_sh_program_start as *const u8;
        let end = &wovenhat_sh_program_end as *const u8;
        core::slice::from_raw_parts(start, end.offset_from(start) as usize)
    };
    let elf = build_stub_elf(stub)?;
    load_elf_with_argv(&elf, &["/bin/sh"])
}

/// Install `/bin/sh` into the VFS.

pub fn install_true_executable() -> bool {
    let stub = unsafe {
        let start = &wovenhat_true_program_start as *const u8;
        let end = &wovenhat_true_program_end as *const u8;
        core::slice::from_raw_parts(start, end.offset_from(start) as usize)
    };
    build_stub_elf(stub).is_some_and(|elf| crate::vfs::create_read_only("/bin/true", &elf).is_ok())
}

pub fn install_false_executable() -> bool {
    let stub = unsafe {
        let start = &wovenhat_false_program_start as *const u8;
        let end = &wovenhat_false_program_end as *const u8;
        core::slice::from_raw_parts(start, end.offset_from(start) as usize)
    };
    build_stub_elf(stub).is_some_and(|elf| crate::vfs::create_read_only("/bin/false", &elf).is_ok())
}


pub fn install_ls_executable() -> bool {
    let stub = unsafe {
        let start = &wovenhat_ls_program_start as *const u8;
        let end = &wovenhat_ls_program_end as *const u8;
        core::slice::from_raw_parts(start, end.offset_from(start) as usize)
    };
    build_stub_elf(stub).is_some_and(|elf| crate::vfs::create_read_only("/bin/ls", &elf).is_ok())
}

pub fn install_sleep_executable() -> bool {
    let stub = unsafe {
        let start = &wovenhat_sleepbin_program_start as *const u8;
        let end = &wovenhat_sleepbin_program_end as *const u8;
        core::slice::from_raw_parts(start, end.offset_from(start) as usize)
    };
    build_stub_elf(stub).is_some_and(|elf| crate::vfs::create_read_only("/bin/sleep", &elf).is_ok())
}

pub fn install_cat_executable() -> bool {
    let stub = unsafe {
        let start = &wovenhat_cat_program_start as *const u8;
        let end = &wovenhat_cat_program_end as *const u8;
        core::slice::from_raw_parts(start, end.offset_from(start) as usize)
    };
    build_stub_elf(stub).is_some_and(|elf| crate::vfs::create_read_only("/bin/cat", &elf).is_ok())
}

pub fn install_echo_executable() -> bool {
    let stub = unsafe {
        let start = &wovenhat_echo_program_start as *const u8;
        let end = &wovenhat_echo_program_end as *const u8;
        core::slice::from_raw_parts(start, end.offset_from(start) as usize)
    };
    build_stub_elf(stub).is_some_and(|elf| crate::vfs::create_read_only("/bin/echo", &elf).is_ok())
}

pub fn install_shell_executable() -> bool {
    let stub = unsafe {
        let start = &wovenhat_sh_program_start as *const u8;
        let end = &wovenhat_sh_program_end as *const u8;
        core::slice::from_raw_parts(start, end.offset_from(start) as usize)
    };
    build_stub_elf(stub).is_some_and(|elf| crate::vfs::create_read_only("/bin/sh", &elf).is_ok())
}
pub fn load_elf(bytes: &[u8]) -> Option<UserProgram> {
    let image = crate::elf::parse(bytes).ok()?;
    let page_table = paging::create_user_address_space(image.entry)?;
    let mut mappings = [UserMapping::EMPTY; MAX_ELF_SEGMENTS];
    let mut mapping_count = 0;

    let stack_base = USER_REGION_START.checked_add(USER_STACK_OFFSET)?;
    let stack = UserStack::new(stack_base);
    for segment in image.segments() {
        let mapping_end = segment
            .mapping_start
            .checked_add(segment.mapping_size as u64)?;
        if segment.mapping_start < USER_REGION_START || mapping_end > stack.guard_base {
            release_partial(page_table, &mappings[..mapping_count]);
            return None;
        }
        if mapping_count == mappings.len()
            || paging::map_user_range_in(
                page_table,
                segment.mapping_start,
                segment.mapping_size,
                true,
                false,
            )
            .is_err()
        {
            release_partial(page_table, &mappings[..mapping_count]);
            return None;
        }
        mappings[mapping_count] = UserMapping {
            start: segment.mapping_start,
            size: segment.mapping_size,
            writable: segment.writable,
            executable: segment.executable,
        };
        mapping_count += 1;

        let file_end = segment.file_offset.checked_add(segment.file_size)?;
        let memory_end = segment
            .virtual_address
            .checked_add(segment.memory_size as u64)?;
        if memory_end
            > segment
                .mapping_start
                .checked_add(segment.mapping_size as u64)?
            || paging::zero_user_range_in(page_table, segment.mapping_start, segment.mapping_size)
                .is_err()
            || paging::write_user_bytes(
                page_table,
                segment.virtual_address,
                bytes.get(segment.file_offset..file_end)?,
            )
            .is_err()
            || paging::protect_user_range_in(
                page_table,
                segment.mapping_start,
                segment.mapping_size,
                segment.writable,
                segment.executable,
            )
            .is_err()
        {
            release_partial(page_table, &mappings[..mapping_count]);
            return None;
        }
    }

    if paging::map_user_range_in(page_table, stack_base, UserStack::SIZE, true, false).is_err() {
        release_partial(page_table, &mappings[..mapping_count]);
        return None;
    }
    if paging::zero_user_range_in(page_table, stack_base, UserStack::SIZE).is_err() {
        release_with_stack(page_table, stack_base, &mappings[..mapping_count]);
        return None;
    }

    if !paging::user_range_is_unmapped_in(page_table, stack.guard_base, UserStack::GUARD_SIZE)
        || !paging::user_range_has_protection_in(page_table, stack.base, stack.size, true, false)
    {
        release_with_stack(page_table, stack_base, &mappings[..mapping_count]);
        return None;
    }

    // Default argv so every loaded image starts with a valid C-style stack.
    let stack_top = setup_argv_stack(page_table, stack, &["a.out"])?;

    Some(UserProgram {
        image: UserImage {
            entry: image.entry,
            stack_top,
            image_size: bytes.len() as u64,
            load_segments: image.segment_count(),
        },
        stack,
        address_space: AddressSpace {
            paging: page_table,
            stack_base,
            mappings,
            mapping_count,
        },
    })
}

/// Load an ELF and place a custom argv vector on the user stack.
pub fn load_elf_with_argv(bytes: &[u8], args: &[&str]) -> Option<UserProgram> {
    let mut program = load_elf(bytes)?;
    // Rebuild argv over the default one.
    let stack_top = setup_argv_stack(program.address_space.paging(), program.stack, args)?;
    program.image.stack_top = stack_top;
    Some(program)
}

fn release_with_stack(page_table: paging::AddressSpace, stack_base: u64, mappings: &[UserMapping]) {
    let mut ranges = [(0, 0); MAX_ELF_SEGMENTS + 1];
    ranges[0] = (stack_base, UserStack::SIZE);
    for (range, mapping) in ranges[1..].iter_mut().zip(mappings) {
        *range = (mapping.start, mapping.size);
    }
    let _ = paging::destroy_user_address_space(page_table, &ranges[..mappings.len() + 1]);
}
fn release_partial(page_table: paging::AddressSpace, mappings: &[UserMapping]) {
    if mappings.is_empty() {
        let _ = paging::discard_empty_user_address_space(page_table);
        return;
    }
    let mut ranges = [(0, 0); MAX_ELF_SEGMENTS];
    for (range, mapping) in ranges.iter_mut().zip(mappings) {
        *range = (mapping.start, mapping.size);
    }
    let _ = paging::destroy_user_address_space(page_table, &ranges[..mappings.len()]);
}

pub fn map_anonymous(
    address_space: AddressSpace,
    slot: usize,
    size: usize,
    writable: bool,
) -> Option<AnonymousMapping> {
    if slot >= MAX_ANONYMOUS_MAPPINGS
        || size == 0
        || size > USER_MMAP_MAX_SIZE
        || !size.is_multiple_of(4096)
    {
        return None;
    }
    let address = USER_MMAP_START.checked_add((slot as u64).checked_mul(USER_MMAP_STRIDE)?)?;
    if paging::map_user_range_in(address_space.paging, address, size, writable, false).is_err() {
        return None;
    }
    if paging::zero_user_range_in(address_space.paging, address, size).is_err() {
        let _ = paging::unmap_user_range_in(address_space.paging, address, size);
        return None;
    }
    Some(AnonymousMapping {
        address,
        size,
        writable,
    })
}

pub fn unmap_anonymous(address_space: AddressSpace, mapping: AnonymousMapping) -> bool {
    paging::unmap_user_range_in(address_space.paging, mapping.address, mapping.size).is_ok()
}
pub fn destroy_process_address_space(
    address_space: AddressSpace,
    anonymous: [Option<AnonymousMapping>; MAX_ANONYMOUS_MAPPINGS],
) -> bool {
    for mapping in anonymous.into_iter().flatten() {
        if !unmap_anonymous(address_space, mapping) {
            return false;
        }
    }
    destroy(address_space)
}
pub fn destroy(address_space: AddressSpace) -> bool {
    let mut ranges = [(0, 0); MAX_ELF_SEGMENTS + 1];
    ranges[0] = (address_space.stack_base, UserStack::SIZE);
    for (range, mapping) in ranges[1..]
        .iter_mut()
        .zip(&address_space.mappings[..address_space.mapping_count])
    {
        *range = (mapping.start, mapping.size);
    }
    paging::destroy_user_address_space(
        address_space.paging,
        &ranges[..address_space.mapping_count + 1],
    )
    .is_ok()
}

pub fn clone_address_space(
    source: AddressSpace,
    anonymous: &[Option<AnonymousMapping>; MAX_ANONYMOUS_MAPPINGS],
) -> Option<AddressSpace> {
    let destination_paging = paging::create_user_address_space(source.stack_base)?;
    let destination = AddressSpace {
        paging: destination_paging,
        stack_base: source.stack_base,
        mappings: source.mappings,
        mapping_count: source.mapping_count,
    };
    let mut completed = [(0_u64, 0_usize); MAX_ELF_SEGMENTS + 1 + MAX_ANONYMOUS_MAPPINGS];
    let mut completed_count = 0;

    // Copy-on-write: share physical frames and mark writable ranges read-only.
    for mapping in &source.mappings[..source.mapping_count] {
        if paging::share_user_range_in(
            source.paging,
            destination.paging,
            mapping.start,
            mapping.size,
            mapping.writable,
            mapping.executable,
        )
        .is_err()
        {
            release_clone(destination.paging, &completed[..completed_count]);
            return None;
        }
        completed[completed_count] = (mapping.start, mapping.size);
        completed_count += 1;
    }
    if paging::share_user_range_in(
        source.paging,
        destination.paging,
        source.stack_base,
        UserStack::SIZE,
        true,
        false,
    )
    .is_err()
    {
        release_clone(destination.paging, &completed[..completed_count]);
        return None;
    }
    completed[completed_count] = (source.stack_base, UserStack::SIZE);
    completed_count += 1;

    for mapping in anonymous.iter().flatten() {
        if paging::share_user_range_in(
            source.paging,
            destination.paging,
            mapping.address,
            mapping.size,
            mapping.writable,
            false,
        )
        .is_err()
        {
            release_clone(destination.paging, &completed[..completed_count]);
            return None;
        }
        completed[completed_count] = (mapping.address, mapping.size);
        completed_count += 1;
    }
    Some(destination)
}

fn release_clone(address_space: paging::AddressSpace, ranges: &[(u64, usize)]) {
    if ranges.is_empty() {
        let _ = paging::discard_empty_user_address_space(address_space);
    } else {
        let _ = paging::destroy_user_address_space(address_space, ranges);
    }
}
fn build_stub_elf(stub: &[u8]) -> Option<alloc::vec::Vec<u8>> {
    const PAYLOAD_OFFSET: usize = 4096;
    const PROGRAM_HEADER_OFFSET: usize = 64;
    let total_size = PAYLOAD_OFFSET.checked_add(stub.len())?;
    let mut bytes = alloc::vec![0_u8; total_size];
    bytes[..4].copy_from_slice(b"\x7fELF");
    bytes[4] = 2;
    bytes[5] = 1;
    bytes[6] = 1;
    write_u16(&mut bytes, 16, 2)?;
    write_u16(&mut bytes, 18, 0x3e)?;
    write_u32(&mut bytes, 20, 1)?;
    write_u64(&mut bytes, 24, USER_REGION_START)?;
    write_u64(&mut bytes, 32, PROGRAM_HEADER_OFFSET as u64)?;
    write_u16(&mut bytes, 52, 64)?;
    write_u16(&mut bytes, 54, 56)?;
    write_u16(&mut bytes, 56, 1)?;

    write_u32(&mut bytes, PROGRAM_HEADER_OFFSET, 1)?;
    write_u32(&mut bytes, PROGRAM_HEADER_OFFSET + 4, 5)?;
    write_u64(&mut bytes, PROGRAM_HEADER_OFFSET + 8, PAYLOAD_OFFSET as u64)?;
    write_u64(&mut bytes, PROGRAM_HEADER_OFFSET + 16, USER_REGION_START)?;
    write_u64(&mut bytes, PROGRAM_HEADER_OFFSET + 32, stub.len() as u64)?;
    write_u64(
        &mut bytes,
        PROGRAM_HEADER_OFFSET + 40,
        USER_CODE_SIZE as u64,
    )?;
    write_u64(&mut bytes, PROGRAM_HEADER_OFFSET + 48, 4096)?;
    bytes[PAYLOAD_OFFSET..].copy_from_slice(stub);
    Some(bytes)
}

fn write_u16(bytes: &mut [u8], offset: usize, value: u16) -> Option<()> {
    bytes
        .get_mut(offset..offset + 2)?
        .copy_from_slice(&value.to_le_bytes());
    Some(())
}

fn write_u32(bytes: &mut [u8], offset: usize, value: u32) -> Option<()> {
    bytes
        .get_mut(offset..offset + 4)?
        .copy_from_slice(&value.to_le_bytes());
    Some(())
}

fn write_u64(bytes: &mut [u8], offset: usize, value: u64) -> Option<()> {
    bytes
        .get_mut(offset..offset + 8)?
        .copy_from_slice(&value.to_le_bytes());
    Some(())
}
