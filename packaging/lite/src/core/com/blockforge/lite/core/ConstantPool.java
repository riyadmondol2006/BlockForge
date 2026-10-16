package com.blockforge.lite.core;

import java.nio.charset.Charset;

/**
 * The cheap first filter: walks a class file's constant pool (without ASM, without making objects) and says which of
 * the wanted texts it contains. Most classes are ruled out here in a few microseconds.
 */
final class ConstantPool {
    private static final Charset UTF8 = Charset.forName("UTF-8");
    private final byte[][] wanted;

    ConstantPool(String[] texts) {
        wanted = new byte[texts.length][];
        for (int i = 0; i < texts.length; i++) {
            // the markers are plain ASCII, which is the same in the class file's modified UTF-8
            wanted[i] = texts[i].getBytes(UTF8);
        }
    }

    /**
     * Sets found[i] for every wanted text the constant pool has. Returns false when the bytes are not a class file
     * this code understands (then nothing should be done with them).
     */
    boolean scan(byte[] b, boolean[] found) {
        try {
            if (b.length < 10 || (b[0] & 0xFF) != 0xCA || (b[1] & 0xFF) != 0xFE || (b[2] & 0xFF) != 0xBA
                    || (b[3] & 0xFF) != 0xBE) {
                return false;
            }
            int count = u2(b, 8);
            int p = 10;
            for (int i = 1; i < count; i++) {
                int tag = b[p];
                switch (tag) {
                    case 1: { // Utf8
                        int len = u2(b, p + 1);
                        match(b, p + 3, len, found);
                        p += 3 + len;
                        break;
                    }
                    case 3: case 4: case 9: case 10: case 11: case 12: case 17: case 18:
                        p += 5;
                        break;
                    case 5: case 6: // Long, Double take two slots
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
                        return false;
                }
            }
            return p <= b.length;
        } catch (ArrayIndexOutOfBoundsException e) {
            return false;
        }
    }

    private void match(byte[] b, int start, int len, boolean[] found) {
        for (int k = 0; k < wanted.length; k++) {
            byte[] w = wanted[k];
            if (found[k] || w.length != len) {
                continue;
            }
            int j = 0;
            while (j < len && b[start + j] == w[j]) {
                j++;
            }
            if (j == len) {
                found[k] = true;
            }
        }
    }

    private static int u2(byte[] b, int p) {
        return ((b[p] & 0xFF) << 8) | (b[p + 1] & 0xFF);
    }
}
