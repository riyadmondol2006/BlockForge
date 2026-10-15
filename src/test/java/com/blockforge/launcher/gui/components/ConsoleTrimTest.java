/*
 * BlockForge - a fork of ATLauncher (https://github.com/ATLauncher/ATLauncher)
 *
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU General Public License as published by
 * the Free Software Foundation, either version 3 of the License, or
 * (at your option) any later version.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
 * GNU General Public License for more details.
 *
 * You should have received a copy of the GNU General Public License
 * along with this program. If not, see <http://www.gnu.org/licenses/>.
 */
package com.blockforge.launcher.gui.components;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertTrue;

import javax.swing.text.DefaultStyledDocument;
import javax.swing.text.SimpleAttributeSet;

import org.junit.jupiter.api.Test;

class ConsoleTrimTest {
    private static DefaultStyledDocument withLines(int count) throws Exception {
        DefaultStyledDocument doc = new DefaultStyledDocument();
        for (int i = 0; i < count; i++) {
            doc.insertString(doc.getLength(), "line " + i + "\n", new SimpleAttributeSet());
        }
        return doc;
    }

    @Test
    void leavesASmallLogAlone() throws Exception {
        DefaultStyledDocument doc = withLines(15);
        assertEquals(0, Console.trimToLines(doc, 10, 5));
        assertTrue(doc.getText(0, doc.getLength()).startsWith("line 0\n"));
    }

    @Test
    void dropsTheOldestLinesOnceOverTheSlack() throws Exception {
        DefaultStyledDocument doc = withLines(16);
        assertEquals(6, Console.trimToLines(doc, 10, 5));
        String text = doc.getText(0, doc.getLength());
        assertTrue(text.startsWith("line 6\n"), text);
        assertTrue(text.endsWith("line 15\n"), text);
        assertEquals(10, text.split("\n").length);
    }

    @Test
    void keepsWholeLinesWhenALineIsHalfWritten() throws Exception {
        DefaultStyledDocument doc = withLines(20);
        // a timestamp is written before the rest of its line
        doc.insertString(doc.getLength(), "[12:00:00] ", new SimpleAttributeSet());
        Console.trimToLines(doc, 10, 0);
        String text = doc.getText(0, doc.getLength());
        assertTrue(text.startsWith("line 10\n"), text);
        assertTrue(text.endsWith("line 19\n[12:00:00] "), text);
    }

    @Test
    void stackTracesWrittenInOneGoCountEveryLine() throws Exception {
        DefaultStyledDocument doc = new DefaultStyledDocument();
        StringBuilder trace = new StringBuilder();
        for (int i = 0; i < 100; i++) {
            trace.append("at frame ").append(i).append('\n');
        }
        doc.insertString(0, trace.toString(), new SimpleAttributeSet());
        assertEquals(70, Console.trimToLines(doc, 30, 10));
        assertTrue(doc.getText(0, doc.getLength()).startsWith("at frame 70\n"));
    }
}
