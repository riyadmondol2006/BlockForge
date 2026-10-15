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
package com.blockforge.launcher.gui.modern;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;

import org.junit.jupiter.api.Test;

class UpdateDialogNotesTest {
    @Test
    void turnsMarkdownIntoPlainText() {
        String markdown = "## What's Changed\r\n\r\n* **Faster** start by @someone in [#12](https://x/pull/12)\r\n"
            + "- Fix `crash` on launch\r\n\r\n\r\n\r\n<!-- hidden -->\r\n**Full Changelog**: https://x/compare";
        String text = UpdateDialog.plainNotes(markdown);

        assertEquals("What's Changed\n\n• Faster start by @someone in #12\n• Fix crash on launch\n\n"
            + "Full Changelog: https://x/compare", text);
        assertFalse(text.contains("\r"));
    }

    @Test
    void emptyNotesStillSaySomething() {
        assertFalse(UpdateDialog.plainNotes(null).isEmpty());
        assertFalse(UpdateDialog.plainNotes("  \n ").isEmpty());
    }
}
