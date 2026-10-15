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

import java.awt.Dimension;
import java.awt.Point;
import java.awt.Rectangle;

import org.junit.jupiter.api.Test;

class PopupsTest {
    private static final Rectangle WINDOW = new Rectangle(100, 50, 800, 600);
    private static final Dimension MENU = new Dimension(250, 400);

    @Test
    void opensBelowWhenThereIsRoom() {
        assertEquals(new Point(300, 150), Popups.place(new Point(300, 150), 32, MENU, WINDOW));
    }

    @Test
    void flipsAboveTheButtonNearTheBottom() {
        // the button is 32 tall and ends at y 550, so the menu ends where the button starts
        assertEquals(new Point(300, 118), Popups.place(new Point(300, 550), 32, MENU, WINDOW));
    }

    @Test
    void flipsAboveARightClickNearTheBottom() {
        assertEquals(new Point(300, 200), Popups.place(new Point(300, 600), 0, MENU, WINDOW));
    }

    @Test
    void staysInsideWhenItFitsNeitherWay() {
        // the window bottom is at 650: there is room for 400 neither below 400 nor above 368
        assertEquals(new Point(300, 250), Popups.place(new Point(300, 400), 32, MENU, WINDOW));
    }

    @Test
    void startsAtTheTopWhenTallerThanTheWindow() {
        assertEquals(new Point(300, 50), Popups.place(new Point(300, 400), 32, new Dimension(250, 900), WINDOW));
    }

    @Test
    void keepsInsideTheSides() {
        assertEquals(new Point(650, 150), Popups.place(new Point(820, 150), 32, MENU, WINDOW));
        assertEquals(new Point(100, 150), Popups.place(new Point(40, 150), 32, MENU, WINDOW));
    }
}
