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
        return withFallback(App.THEME.getNormalFont(), Font.PLAIN).deriveFont(Font.PLAIN, size);
    }

    private static Font plainBase;
    private static Font plainSource;
    private static Font boldBase;
    private static Font boldSource;

    /**
     * Wraps a bundled font so characters it doesn't have (Chinese, Japanese, Korean, Arabic...) are drawn with the
     * system's fonts instead of boxes. Swing's StyleContext builds such a composite font; FlatLaf does the same.
     */
    private static synchronized Font withFallback(Font source, int style) {
        boolean plain = style == Font.PLAIN;
        if (plain ? source == plainSource : source == boldSource) {
            return plain ? plainBase : boldBase;
        }
        Font composite = source;
        try {
            Font c = javax.swing.text.StyleContext.getDefaultStyleContext().getFont(source.getFamily(), style,
                Math.max(1, source.getSize()));
            if (c != null && c.getFamily().equals(source.getFamily())) {
                composite = c;
            }
        } catch (Exception ignored) {
            // keep the bundled font; fontFor still switches to Dialog for text it can't draw
        }
        if (plain) {
            plainSource = source;
            plainBase = composite;
        } else {
            boldSource = source;
            boldBase = composite;
        }
        return composite;
    }

    /**
     * The interface font for showing {@code text}. The theme font already falls back to system fonts (see
     * withFallback); if some characters are still missing and the system's "Dialog" font can draw all of them, that
     * is used instead.
     */
    public static Font fontFor(String text, float size, boolean bold) {
        Font font = bold ? bold(size) : font(size);
        if (text != null && !text.isEmpty() && font.canDisplayUpTo(text) != -1) {
            Font dialog = new Font(Font.DIALOG, bold ? Font.BOLD : Font.PLAIN, 12).deriveFont(size);
            // only worth giving up Roboto when the system font can really draw all of it
            if (dialog.canDisplayUpTo(text) == -1) {
                return dialog;
            }
        }
        return font;
    }

    public static Font bold(float size) {
        return withFallback(App.THEME.getBoldFont(), Font.BOLD).deriveFont(Font.BOLD, size);
    }
}
