package com.blockforge.lite.core;

import java.nio.charset.Charset;
import java.util.ArrayList;
import java.util.List;

/**
 * Finds the methods of a class file that may call a given method, by looking at the raw bytes: which constant pool
 * entries name that method, and which methods' code has an invoke instruction with one of those entries. It may
 * report a method that only contains such bytes by chance; the caller reads those with ASM and checks. It never
 * misses one. This is much cheaper than letting ASM read every method of a big class.
 */
final class ClassFile {
    private static final Charset UTF8 = Charset.forName("UTF-8");

    private ClassFile() {}

    private static final int METHOD = 0;
    private static final int FIELD = 1;
    private static final int STRING = 2;

    /**
     * name + descriptor of each method that may call owner.name desc (name null: any name); empty when none or the
     * bytes are odd.
     */
    static List<String> methodsCalling(byte[] b, String owner, String name, String desc) {
        return methodsUsing(b, owner, name, desc, METHOD);
    }

    /** The same for the methods that may read or write the field owner.name desc. */
    static List<String> methodsUsingField(byte[] b, String owner, String name, String desc) {
        return methodsUsing(b, owner, name, desc, FIELD);
    }

    /** The same for the methods that may load this string constant (ldc). */
    static List<String> methodsUsingString(byte[] b, String text) {
        return methodsUsing(b, null, null, text, STRING);
    }

    private static List<String> methodsUsing(byte[] b, String owner, String name, String desc, int kind) {
        int firstOp = kind == FIELD ? 0xB2 : kind == METHOD ? 0xB6 : 0x12;
        int lastOp = kind == FIELD ? 0xB5 : kind == METHOD ? 0xB9 : 0x13;
        List<String> out = new ArrayList<String>();
        try {
            int count = u2(b, 8);
            int[] off = new int[count];
            int p = 10;
            for (int i = 1; i < count; i++) {
                off[i] = p;
                int tag = b[p];
                switch (tag) {
                    case 1:
                        p += 3 + u2(b, p + 1);
                        break;
                    case 3: case 4: case 9: case 10: case 11: case 12: case 17: case 18:
                        p += 5;
                        break;
                    case 5: case 6:
                        p += 9;
                        i++;
                        break;
                    case 7: case 8: case 16: case 19: case 20:
                        p += 3;
                        break;
                    case 15:
                        p += 4;
                        break;
                    default:
                        return out;
                }
            }
            byte[] o = owner == null ? null : owner.getBytes(UTF8);
            byte[] n = name == null ? null : name.getBytes(UTF8);
            byte[] d = desc.getBytes(UTF8);
            boolean[] wanted = new boolean[count];
            boolean any = false;
            for (int i = 1; i < count; i++) {
                int tag = b[off[i]];
                if (kind == STRING) {
                    if (tag == 8 && utf8Is(b, off, u2(b, off[i] + 1), d)) {
                        wanted[i] = true;
                        any = true;
                    }
                    continue;
                }
                if (kind == FIELD ? tag != 9 : tag != 10 && tag != 11) {
                    continue;
                }
                int cls = u2(b, off[i] + 1);
                int nat = u2(b, off[i] + 3);
                if (utf8Is(b, off, u2(b, off[cls] + 1), o) && (n == null || utf8Is(b, off, u2(b, off[nat] + 1), n))
                        && utf8Is(b, off, u2(b, off[nat] + 3), d)) {
                    wanted[i] = true;
                    any = true;
                }
            }
            if (!any) {
                return out;
            }
            p += 6; // access, this, super
            p += 2 + 2 * u2(b, p); // interfaces
            int fields = u2(b, p);
            p += 2;
            for (int f = 0; f < fields; f++) {
                p = skipAttributes(b, p + 6);
            }
            int methods = u2(b, p);
            p += 2;
            for (int m = 0; m < methods; m++) {
                int nameIndex = u2(b, p + 2);
                int descIndex = u2(b, p + 4);
                int attrs = u2(b, p + 6);
                p += 8;
                boolean calls = false;
                for (int a = 0; a < attrs; a++) {
                    int len = u4(b, p + 2);
                    if (utf8Is(b, off, u2(b, p), CODE)) {
                        int codeLen = u4(b, p + 10);
                        int start = p + 14;
                        for (int k = start; k + 1 < start + codeLen && !calls; k++) {
                            int op = b[k] & 0xFF;
                            if (op < firstOp || op > lastOp) {
                                continue;
                            }
                            // ldc has a one-byte index, every other instruction here a two-byte one
                            int index = op == 0x12 ? b[k + 1] & 0xFF : k + 2 < start + codeLen ? u2(b, k + 1) : 0;
                            if (wanted[index % count]) {
                                calls = true;
                            }
                        }
                    }
                    p += 6 + len;
                }
                if (calls) {
                    out.add(utf8(b, off, nameIndex) + utf8(b, off, descIndex));
                }
            }
        } catch (RuntimeException e) {
            // not a class file this understands: nothing found
            out.clear();
        }
        return out;
    }

    private static final byte[] CODE = {'C', 'o', 'd', 'e'};

    private static int skipAttributes(byte[] b, int p) {
        int attrs = u2(b, p);
        p += 2;
        for (int a = 0; a < attrs; a++) {
            p += 6 + u4(b, p + 2);
        }
        return p;
    }

    private static boolean utf8Is(byte[] b, int[] off, int index, byte[] want) {
        int p = off[index];
        if (b[p] != 1 || u2(b, p + 1) != want.length) {
            return false;
        }
        for (int i = 0; i < want.length; i++) {
            if (b[p + 3 + i] != want[i]) {
                return false;
            }
        }
        return true;
    }

    private static String utf8(byte[] b, int[] off, int index) {
        int p = off[index];
        // the names of methods and descriptors are almost always ASCII; modified UTF-8 is the same as UTF-8 for
        // everything but the zero character and characters outside the BMP, which don't occur in method names
        return new String(b, p + 3, u2(b, p + 1), UTF8);
    }

    private static int u2(byte[] b, int p) {
        return ((b[p] & 0xFF) << 8) | (b[p + 1] & 0xFF);
    }

    private static int u4(byte[] b, int p) {
        return ((b[p] & 0xFF) << 24) | ((b[p + 1] & 0xFF) << 16) | ((b[p + 2] & 0xFF) << 8) | (b[p + 3] & 0xFF);
    }
}
