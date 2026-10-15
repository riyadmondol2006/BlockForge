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

import java.awt.BasicStroke;
import java.awt.Color;
import java.awt.Graphics2D;
import java.awt.RenderingHints;
import java.awt.geom.Arc2D;
import java.awt.geom.Ellipse2D;
import java.awt.geom.Line2D;
import java.awt.geom.Path2D;
import java.awt.geom.RoundRectangle2D;

import javax.swing.Icon;

/**
 * Vector icons drawn in code on a 24x24 grid, so they stay crisp at any size and take any colour.
 */
public enum Icons {
    HOME, LIBRARY, EXPLORE, PERSON, SETTINGS, PLAY, PLUS, MORE, FOLDER, TRASH, SEARCH, CLOSE, DOWNLOAD, TERMINAL,
    SERVER, CUBE, INFO, REFRESH, CHECK, EXTERNAL, EDIT, IMPORT, WARNING, PUZZLE, IMAGE, SPARKLE, BOLT;

    /**
     * Paints this icon into the square at (x, y) with the given size.
     */
    public void paint(Graphics2D g0, float x, float y, float size, Color color) {
        Graphics2D g = (Graphics2D) g0.create();
        g.setRenderingHint(RenderingHints.KEY_ANTIALIASING, RenderingHints.VALUE_ANTIALIAS_ON);
        g.setRenderingHint(RenderingHints.KEY_STROKE_CONTROL, RenderingHints.VALUE_STROKE_PURE);
        g.translate(x, y);
        float scale = size / 24f;
        g.scale(scale, scale);
        g.setColor(color);
        g.setStroke(new BasicStroke(2f, BasicStroke.CAP_ROUND, BasicStroke.JOIN_ROUND));

        switch (this) {
            case HOME: {
                Path2D roof = new Path2D.Float();
                roof.moveTo(3, 11);
                roof.lineTo(12, 3.5);
                roof.lineTo(21, 11);
                g.draw(roof);
                Path2D walls = new Path2D.Float();
                walls.moveTo(5.5, 9.5);
                walls.lineTo(5.5, 20);
                walls.lineTo(10, 20);
                walls.lineTo(10, 14);
                walls.lineTo(14, 14);
                walls.lineTo(14, 20);
                walls.lineTo(18.5, 20);
                walls.lineTo(18.5, 9.5);
                g.draw(walls);
                break;
            }
            case LIBRARY:
                g.draw(new RoundRectangle2D.Float(3.5f, 3.5f, 7f, 7f, 2.5f, 2.5f));
                g.draw(new RoundRectangle2D.Float(13.5f, 3.5f, 7f, 7f, 2.5f, 2.5f));
                g.draw(new RoundRectangle2D.Float(3.5f, 13.5f, 7f, 7f, 2.5f, 2.5f));
                g.draw(new RoundRectangle2D.Float(13.5f, 13.5f, 7f, 7f, 2.5f, 2.5f));
                break;
            case EXPLORE: {
                g.draw(new Ellipse2D.Float(3, 3, 18, 18));
                Path2D needle = new Path2D.Float();
                needle.moveTo(15.5, 8.5);
                needle.lineTo(13.4, 13.4);
                needle.lineTo(8.5, 15.5);
                needle.lineTo(10.6, 10.6);
                needle.closePath();
                g.fill(needle);
                g.draw(needle);
                break;
            }
            case PERSON: {
                g.draw(new Ellipse2D.Float(8, 3.5f, 8, 8));
                Path2D body = new Path2D.Float();
                body.moveTo(4.5, 20.5);
                body.curveTo(4.5, 16, 8, 14.5, 12, 14.5);
                body.curveTo(16, 14.5, 19.5, 16, 19.5, 20.5);
                g.draw(body);
                break;
            }
            case SETTINGS: {
                Path2D gear = new Path2D.Float();
                int teeth = 8;
                for (int i = 0; i < teeth * 2; i++) {
                    double a0 = Math.PI * 2 * i / (teeth * 2) - Math.PI / (teeth * 4);
                    double a1 = a0 + Math.PI * 2 / (teeth * 2);
                    double r = i % 2 == 0 ? 9.5 : 7.4;
                    double px0 = 12 + Math.cos(a0) * r;
                    double py0 = 12 + Math.sin(a0) * r;
                    double px1 = 12 + Math.cos(a1) * r;
                    double py1 = 12 + Math.sin(a1) * r;
                    if (i == 0) {
                        gear.moveTo(px0, py0);
                    } else {
                        gear.lineTo(px0, py0);
                    }
                    gear.lineTo(px1, py1);
                }
                gear.closePath();
                g.setStroke(new BasicStroke(1.8f, BasicStroke.CAP_ROUND, BasicStroke.JOIN_ROUND));
                g.draw(gear);
                g.draw(new Ellipse2D.Float(8.8f, 8.8f, 6.4f, 6.4f));
                break;
            }
            case PLAY: {
                Path2D tri = new Path2D.Float();
                tri.moveTo(7.5, 4.5);
                tri.lineTo(19.5, 12);
                tri.lineTo(7.5, 19.5);
                tri.closePath();
                g.fill(tri);
                g.draw(tri);
                break;
            }
            case PLUS:
                g.draw(new Line2D.Float(12, 5, 12, 19));
                g.draw(new Line2D.Float(5, 12, 19, 12));
                break;
            case MORE:
                g.fill(new Ellipse2D.Float(10.2f, 3.2f, 3.6f, 3.6f));
                g.fill(new Ellipse2D.Float(10.2f, 10.2f, 3.6f, 3.6f));
                g.fill(new Ellipse2D.Float(10.2f, 17.2f, 3.6f, 3.6f));
                break;
            case FOLDER: {
                Path2D f = new Path2D.Float();
                f.moveTo(3.5, 6.5);
                f.lineTo(3.5, 18.5);
                f.quadTo(3.5, 19.5, 4.5, 19.5);
                f.lineTo(19.5, 19.5);
                f.quadTo(20.5, 19.5, 20.5, 18.5);
                f.lineTo(20.5, 9);
                f.quadTo(20.5, 8, 19.5, 8);
                f.lineTo(11.5, 8);
                f.lineTo(9.5, 5.5);
                f.lineTo(4.5, 5.5);
                f.quadTo(3.5, 5.5, 3.5, 6.5);
                g.draw(f);
                break;
            }
            case TRASH: {
                g.draw(new Line2D.Float(4, 6.5f, 20, 6.5f));
                Path2D lid = new Path2D.Float();
                lid.moveTo(9, 6.5);
                lid.lineTo(9, 4);
                lid.lineTo(15, 4);
                lid.lineTo(15, 6.5);
                g.draw(lid);
                Path2D bin = new Path2D.Float();
                bin.moveTo(6, 6.5);
                bin.lineTo(7, 19.5);
                bin.lineTo(17, 19.5);
                bin.lineTo(18, 6.5);
                g.draw(bin);
                g.draw(new Line2D.Float(10, 10.5f, 10, 16));
                g.draw(new Line2D.Float(14, 10.5f, 14, 16));
                break;
            }
            case SEARCH:
                g.draw(new Ellipse2D.Float(4, 4, 12.5f, 12.5f));
                g.draw(new Line2D.Float(14.5f, 14.5f, 20, 20));
                break;
            case CLOSE:
                g.draw(new Line2D.Float(6, 6, 18, 18));
                g.draw(new Line2D.Float(18, 6, 6, 18));
                break;
            case DOWNLOAD: {
                g.draw(new Line2D.Float(12, 4, 12, 15));
                Path2D head = new Path2D.Float();
                head.moveTo(7, 10.5);
                head.lineTo(12, 15.5);
                head.lineTo(17, 10.5);
                g.draw(head);
                g.draw(new Line2D.Float(5, 20, 19, 20));
                break;
            }
            case TERMINAL: {
                g.draw(new RoundRectangle2D.Float(3, 4.5f, 18, 15, 3.5f, 3.5f));
                Path2D chev = new Path2D.Float();
                chev.moveTo(7, 9.5);
                chev.lineTo(10.5, 12);
                chev.lineTo(7, 14.5);
                g.draw(chev);
                g.draw(new Line2D.Float(12.5f, 15, 17, 15));
                break;
            }
            case SERVER:
                g.draw(new RoundRectangle2D.Float(3.5f, 4, 17, 7, 3, 3));
                g.draw(new RoundRectangle2D.Float(3.5f, 13, 17, 7, 3, 3));
                g.fill(new Ellipse2D.Float(6.6f, 6.6f, 1.8f, 1.8f));
                g.fill(new Ellipse2D.Float(6.6f, 15.6f, 1.8f, 1.8f));
                break;
            case CUBE: {
                Path2D hex = new Path2D.Float();
                hex.moveTo(12, 3);
                hex.lineTo(20, 7.5);
                hex.lineTo(20, 16.5);
                hex.lineTo(12, 21);
                hex.lineTo(4, 16.5);
                hex.lineTo(4, 7.5);
                hex.closePath();
                g.draw(hex);
                Path2D inner = new Path2D.Float();
                inner.moveTo(4, 7.5);
                inner.lineTo(12, 12);
                inner.lineTo(20, 7.5);
                inner.moveTo(12, 12);
                inner.lineTo(12, 21);
                g.draw(inner);
                break;
            }
            case INFO:
                g.draw(new Ellipse2D.Float(3, 3, 18, 18));
                g.draw(new Line2D.Float(12, 11, 12, 16.5f));
                g.fill(new Ellipse2D.Float(11, 6.9f, 2, 2));
                break;
            case REFRESH: {
                g.draw(new Arc2D.Float(4.5f, 4.5f, 15, 15, 40, 280, Arc2D.OPEN));
                Path2D head = new Path2D.Float();
                head.moveTo(17.5, 2.5);
                head.lineTo(17.5, 7.5);
                head.lineTo(12.5, 7.5);
                g.draw(head);
                break;
            }
            case CHECK: {
                Path2D ck = new Path2D.Float();
                ck.moveTo(5, 12.5);
                ck.lineTo(10, 17.5);
                ck.lineTo(19, 7);
                g.draw(ck);
                break;
            }
            case EXTERNAL: {
                Path2D box = new Path2D.Float();
                box.moveTo(10, 5);
                box.lineTo(6, 5);
                box.quadTo(4.5, 5, 4.5, 6.5);
                box.lineTo(4.5, 18);
                box.quadTo(4.5, 19.5, 6, 19.5);
                box.lineTo(17.5, 19.5);
                box.quadTo(19, 19.5, 19, 18);
                box.lineTo(19, 14);
                g.draw(box);
                g.draw(new Line2D.Float(12, 12, 20, 4));
                Path2D arrow = new Path2D.Float();
                arrow.moveTo(14, 4);
                arrow.lineTo(20, 4);
                arrow.lineTo(20, 10);
                g.draw(arrow);
                break;
            }
            case EDIT: {
                Path2D pen = new Path2D.Float();
                pen.moveTo(4, 20);
                pen.lineTo(4.8, 15.6);
                pen.lineTo(16, 4.4);
                pen.quadTo(17, 3.4, 18, 4.4);
                pen.lineTo(19.6, 6);
                pen.quadTo(20.6, 7, 19.6, 8);
                pen.lineTo(8.4, 19.2);
                pen.closePath();
                g.draw(pen);
                break;
            }
            case IMPORT: {
                g.draw(new Line2D.Float(12, 4, 12, 14.5f));
                Path2D head = new Path2D.Float();
                head.moveTo(7.5, 10);
                head.lineTo(12, 14.5);
                head.lineTo(16.5, 10);
                g.draw(head);
                Path2D tray = new Path2D.Float();
                tray.moveTo(4.5, 15);
                tray.lineTo(4.5, 19);
                tray.quadTo(4.5, 20, 5.5, 20);
                tray.lineTo(18.5, 20);
                tray.quadTo(19.5, 20, 19.5, 19);
                tray.lineTo(19.5, 15);
                g.draw(tray);
                break;
            }
            case WARNING: {
                Path2D tri = new Path2D.Float();
                tri.moveTo(12, 3.5);
                tri.lineTo(21.5, 20);
                tri.lineTo(2.5, 20);
                tri.closePath();
                g.draw(tri);
                g.draw(new Line2D.Float(12, 9.5f, 12, 14));
                g.fill(new Ellipse2D.Float(11, 15.6f, 2, 2));
                break;
            }
            case PUZZLE: {
                // a jigsaw piece: a square with a knob on top and one on the right
                Path2D p = new Path2D.Float();
                p.moveTo(4.5, 8);
                p.lineTo(9, 8);
                p.curveTo(8, 4, 14, 4, 13, 8);
                p.lineTo(17.5, 8);
                p.lineTo(17.5, 11);
                p.curveTo(21.5, 10, 21.5, 16, 17.5, 15);
                p.lineTo(17.5, 20);
                p.lineTo(4.5, 20);
                p.closePath();
                g.draw(p);
                break;
            }
            case IMAGE: {
                g.draw(new RoundRectangle2D.Float(3.5f, 4.5f, 17, 15, 3.5f, 3.5f));
                g.draw(new Ellipse2D.Float(7.2f, 7.8f, 3.2f, 3.2f));
                Path2D hills = new Path2D.Float();
                hills.moveTo(4, 17.5);
                hills.lineTo(9.5, 12.5);
                hills.lineTo(13, 15.5);
                hills.lineTo(15.5, 13);
                hills.lineTo(20, 17);
                g.draw(hills);
                break;
            }
            case SPARKLE: {
                Path2D star = new Path2D.Float();
                star.moveTo(10, 3.5);
                star.quadTo(11, 10, 17, 11);
                star.quadTo(11, 12, 10, 18.5);
                star.quadTo(9, 12, 3, 11);
                star.quadTo(9, 10, 10, 3.5);
                star.closePath();
                g.draw(star);
                Path2D small = new Path2D.Float();
                small.moveTo(18, 14.5);
                small.quadTo(18.4, 17.1, 21, 17.5);
                small.quadTo(18.4, 17.9, 18, 20.5);
                small.quadTo(17.6, 17.9, 15, 17.5);
                small.quadTo(17.6, 17.1, 18, 14.5);
                small.closePath();
                g.fill(small);
                break;
            }
            case BOLT: {
                Path2D bolt = new Path2D.Float();
                bolt.moveTo(13.5, 3);
                bolt.lineTo(5.5, 13.5);
                bolt.lineTo(11.5, 13.5);
                bolt.lineTo(10.5, 21);
                bolt.lineTo(18.5, 10.5);
                bolt.lineTo(12.5, 10.5);
                bolt.closePath();
                g.draw(bolt);
                break;
            }
            default:
                break;
        }

        g.dispose();
    }

    /**
     * A Swing icon that renders this icon with a fixed colour.
     */
    public Icon asIcon(int size, Color color) {
        return asIcon(size, () -> color);
    }

    /**
     * A Swing icon whose colour is looked up each time it is painted, so it follows a switch between light and dark
     * themes. Use this for icons that stay on screen.
     */
    public Icon asIcon(int size, java.util.function.Supplier<Color> color) {
        Icons self = this;
        return new Icon() {
            @Override
            public void paintIcon(java.awt.Component c, java.awt.Graphics g, int x, int y) {
                self.paint((Graphics2D) g, x, y, size, color.get());
            }

            @Override
            public int getIconWidth() {
                return size;
            }

            @Override
            public int getIconHeight() {
                return size;
            }
        };
    }
}
