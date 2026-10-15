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

import java.awt.AWTEvent;
import java.awt.Toolkit;
import java.awt.event.KeyEvent;
import java.awt.event.MouseEvent;

/**
 * Focus outlines are only useful for keyboard users, so they only show after the keyboard was used last.
 */
public final class FocusRing {
    private static volatile boolean keyboard;

    static {
        try {
            Toolkit.getDefaultToolkit().addAWTEventListener((AWTEvent event) -> {
                if (event.getID() == KeyEvent.KEY_PRESSED) {
                    keyboard = true;
                } else if (event.getID() == MouseEvent.MOUSE_PRESSED) {
                    keyboard = false;
                }
            }, AWTEvent.KEY_EVENT_MASK | AWTEvent.MOUSE_EVENT_MASK);
        } catch (SecurityException ignored) {
            // without the listener the ring simply never shows
        }
    }

    private FocusRing() {
    }

    /** Whether a focus outline should be drawn right now. */
    public static boolean visible() {
        return keyboard;
    }
}
