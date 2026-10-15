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
import java.awt.Dimension;
import java.awt.Font;
import java.awt.FontMetrics;
import java.awt.Graphics;
import java.awt.Graphics2D;
import java.awt.RenderingHints;
import java.awt.font.LineBreakMeasurer;
import java.awt.font.TextAttribute;
import java.awt.font.TextLayout;
import java.text.AttributedCharacterIterator;
import java.text.AttributedString;
import java.util.function.Supplier;

import javax.swing.JComponent;
import javax.swing.JLabel;

/** Small label factory whose colour is resolved when painting. */
public final class Lbl {
    private Lbl() {
    }

    public static JLabel text(String text, float size, boolean bold, Supplier<Color> color) {
        JLabel l = new JLabel(text) {
            private static final long serialVersionUID = 1L;

            @Override
            public Color getForeground() {
                return color.get();
            }

            @Override
            public Font getFont() {
                return Palette.fontFor(getText(), size, bold);
            }
        };
        l.setOpaque(false);
        return l;
    }

    public static JLabel title(String text) {
        return text(text, 26f, true, Palette::text);
    }

    public static JLabel heading(String text) {
        return text(text, 17f, true, Palette::text);
    }

    public static JLabel body(String text) {
        return text(text, 13f, false, Palette::text);
    }

    public static JLabel dim(String text) {
        return text(text, 13f, false, Palette::textDim);
    }

    /** Wrapped text that is cut off with an ellipsis after a number of lines. */
    public static class Paragraph extends JComponent {
        private static final long serialVersionUID = 1L;

        private String text;
        private final float size;
        private final int maxLines;
        private final Supplier<Color> color;
        private int width;

        public Paragraph(String text, float size, int maxLines, int width, Supplier<Color> color) {
            this.text = text == null ? "" : text;
            this.size = size;
            this.maxLines = maxLines;
            this.width = width;
            this.color = color;
            setOpaque(false);
        }

        /**
         * Wraps to whatever width the layout gives this paragraph instead of the width it was made with, so its
         * height grows or shrinks with the window.
         */
        public Paragraph fillWidth() {
            addComponentListener(new java.awt.event.ComponentAdapter() {
                @Override
                public void componentResized(java.awt.event.ComponentEvent e) {
                    if (getWidth() > 0 && getWidth() != width) {
                        int before = getPreferredSize().height;
                        width = getWidth();
                        if (getPreferredSize().height != before) {
                            revalidate();
                        }
                    }
                }
            });
            return this;
        }

        public void setText(String text) {
            this.text = text == null ? "" : text;
            revalidate();
            repaint();
        }

        private int lineHeight() {
            FontMetrics fm = getFontMetrics(Palette.fontFor(text, size, false));
            return fm.getHeight() + 2;
        }

        @Override
        public Dimension getPreferredSize() {
            if (text.isEmpty()) {
                return new Dimension(width, lineHeight());
            }
            return new Dimension(width, lineHeight() * Math.min(maxLines, Math.max(1, countLines())));
        }

        @Override
        public Dimension getMinimumSize() {
            return new Dimension(0, getPreferredSize().height);
        }

        private int countLines() {
            Graphics2D g = (Graphics2D) new java.awt.image.BufferedImage(1, 1, 1).getGraphics();
            g.setRenderingHint(RenderingHints.KEY_TEXT_ANTIALIASING, RenderingHints.VALUE_TEXT_ANTIALIAS_ON);
            g.setRenderingHint(RenderingHints.KEY_FRACTIONALMETRICS, RenderingHints.VALUE_FRACTIONALMETRICS_ON);
            int lines = 0;
            for (String paragraph : text.split("\n")) {
                if (paragraph.isEmpty()) {
                    lines++;
                    continue;
                }
                AttributedString as = new AttributedString(paragraph);
                as.addAttribute(TextAttribute.FONT, Palette.fontFor(paragraph, size, false));
                AttributedCharacterIterator it = as.getIterator();
                LineBreakMeasurer m = new LineBreakMeasurer(it, g.getFontRenderContext());
                while (m.getPosition() < it.getEndIndex() && lines < maxLines) {
                    m.nextLayout(Math.max(20, width));
                    lines++;
                }
                if (lines >= maxLines) {
                    break;
                }
            }
            g.dispose();
            return lines;
        }

        @Override
        protected void paintComponent(Graphics g0) {
            Graphics2D g = (Graphics2D) g0.create();
            g.setRenderingHint(RenderingHints.KEY_ANTIALIASING, RenderingHints.VALUE_ANTIALIAS_ON);
            g.setRenderingHint(RenderingHints.KEY_TEXT_ANTIALIASING, RenderingHints.VALUE_TEXT_ANTIALIAS_ON);
            g.setRenderingHint(RenderingHints.KEY_FRACTIONALMETRICS, RenderingHints.VALUE_FRACTIONALMETRICS_ON);
            g.setColor(color.get());

            float w = Math.max(20, getWidth());
            float y = 0;
            int lines = 0;
            Font font = Palette.fontFor(text, size, false);
            boolean stop = false;
            String[] paragraphs = text.split("\n");

            for (int p = 0; p < paragraphs.length && !stop; p++) {
                String paragraph = paragraphs[p];
                if (paragraph.isEmpty()) {
                    y += lineHeight();
                    lines++;
                    stop = lines >= maxLines;
                    continue;
                }
                AttributedString as = new AttributedString(paragraph);
                as.addAttribute(TextAttribute.FONT, font);
                AttributedCharacterIterator it = as.getIterator();
                LineBreakMeasurer m = new LineBreakMeasurer(it, g.getFontRenderContext());
                while (m.getPosition() < it.getEndIndex()) {
                    boolean last = lines == maxLines - 1;
                    int start = m.getPosition();
                    TextLayout layout = m.nextLayout(w);
                    int end = m.getPosition();
                    boolean more = end < it.getEndIndex() || p < paragraphs.length - 1;
                    if (last && more) {
                        // fit the remainder on the last line with an ellipsis. No more than what the measurer put
                        // on this line can fit, so start from that instead of measuring the whole rest over and over
                        FontMetrics fm = g.getFontMetrics(font);
                        String cut = paragraph.substring(start, Math.max(start + 1, end));
                        while (cut.length() > 1 && fm.stringWidth(cut + "…") > w) {
                            cut = cut.substring(0, cut.length() - 1);
                        }
                        g.setFont(font);
                        g.drawString(stripTrailing(cut) + "…", 0, y + fm.getAscent());
                        stop = true;
                        break;
                    }
                    layout.draw(g, 0, y + layout.getAscent());
                    y += lineHeight();
                    lines++;
                    if (lines >= maxLines) {
                        stop = true;
                        break;
                    }
                }
            }
            g.dispose();
        }

        /** String.stripTrailing() only exists from Java 11, and the launcher still runs on Java 8 */
        static String stripTrailing(String text) {
            int end = text.length();
            while (end > 0 && Character.isWhitespace(text.charAt(end - 1))) {
                end--;
            }
            return text.substring(0, end);
        }
    }
}
