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

import java.awt.Color;
import java.awt.Font;

import com.blockforge.launcher.App;

/**
 * Colour and type tokens for the BlockForge interface. Every getter resolves at paint time, so the interface follows
 * the light/dark choice of the selected theme without needing to be rebuilt.
 */
public final class Palette {
    private Palette() {
    }

    public static boolean dark() {
        return App.THEME == null || App.THEME.isDark();
    }

    private static Color pick(int dark, int light) {
        return new Color(dark() ? dark : light);
    }

    /** window background */
    public static Color bg() {
        return pick(0x0E1210, 0xF3F6EF);
    }

    /** sidebar background */
    public static Color rail() {
        return pick(0x121712, 0xE8EEE3);
    }

    /** cards and raised surfaces */
    public static Color card() {
        return pick(0x1A201B, 0xFFFFFF);
    }

    public static Color cardHover() {
        return pick(0x222A23, 0xEEF4E9);
    }

    /** inputs and tonal buttons */
    public static Color field() {
        return pick(0x232B24, 0xE2E9DC);
    }

    public static Color fieldHover() {
        return pick(0x2C362D, 0xD5DECE);
    }

    /** outline of text fields and dropdowns, the same as md.fieldBorder in the Material themes */
    public static Color fieldBorder() {
        return pick(0x2E382F, 0xC9D3C2);
    }

    public static Color outline() {
        return pick(0x2E382F, 0xD3DCCC);
    }

    public static Color text() {
        return pick(0xE7ECE4, 0x141A15);
    }

    public static Color textDim() {
        return pick(0x9BA89C, 0x586558);
    }

    public static Color accent() {
        return pick(0x5FD97A, 0x1B8A3C);
    }

    public static Color accentHover() {
        return pick(0x7BE892, 0x23A04A);
    }

    public static Color accentPressed() {
        return pick(0x49C264, 0x15712F);
    }

    public static Color onAccent() {
        return pick(0x04210C, 0xFFFFFF);
    }

    /** selected navigation item, highlighted banners */
    public static Color accentSoft() {
        return pick(0x1D3824, 0xD3EED9);
    }

    public static Color danger() {
        return pick(0xFF8A80, 0xC62828);
    }

    public static Color warn() {
        return pick(0xF5C451, 0x9A6A00);
    }

    public static Color alpha(Color c, int alpha) {
        return new Color(c.getRed(), c.getGreen(), c.getBlue(), alpha);
    }

    public static Color mix(Color a, Color b, float t) {
        return new Color(Math.round(a.getRed() + (b.getRed() - a.getRed()) * t),
            Math.round(a.getGreen() + (b.getGreen() - a.getGreen()) * t),
            Math.round(a.getBlue() + (b.getBlue() - a.getBlue()) * t));
    }

    public static Font font(float size) {
        return App.THEME.getNormalFont().deriveFont(Font.PLAIN, size);
    }

    public static Font bold(float size) {
        return App.THEME.getBoldFont().deriveFont(Font.BOLD, size);
    }
}
