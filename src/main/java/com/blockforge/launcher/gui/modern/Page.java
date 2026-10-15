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

import java.awt.BorderLayout;
import java.awt.Color;

import javax.swing.JPanel;

/** Base class of the screens shown in the main window. */
public abstract class Page extends JPanel {
    private static final long serialVersionUID = 1L;

    protected Page() {
        super(new BorderLayout());
        setOpaque(true);
    }

    @Override
    public Color getBackground() {
        return Palette.bg();
    }

    /** Called every time the page is brought to the front. */
    public void onShow() {
    }

    /**
     * Called when the window throws this page away (for example to rebuild everything in a new language). Anything
     * the page registered outside itself must be let go here, or the old page and its view model live on.
     */
    public void dispose() {
    }
}
