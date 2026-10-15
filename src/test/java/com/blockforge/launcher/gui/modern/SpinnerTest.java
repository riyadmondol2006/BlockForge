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

import static org.junit.jupiter.api.Assertions.assertFalse;

import javax.swing.JPanel;

import org.junit.jupiter.api.Test;

class SpinnerTest {
    @Test
    void doesNotAnimateWhileNotOnScreen() {
        Spinner spinner = new Spinner(24);
        assertFalse(spinner.isAnimating());

        // on a panel that isn't in a window, like a card of a page that hasn't been shown
        JPanel hidden = new JPanel();
        hidden.add(spinner);
        assertFalse(spinner.isAnimating());
        hidden.setVisible(false);
        hidden.setVisible(true);
        assertFalse(spinner.isAnimating());
    }
}
