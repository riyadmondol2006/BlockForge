/*
 * ATLauncher - https://github.com/ATLauncher/ATLauncher
 * Copyright (C) 2013-2022 ATLauncher
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

import java.awt.Color;

import javax.swing.JTextPane;
import javax.swing.UIManager;
import javax.swing.text.AbstractDocument;
import javax.swing.text.BadLocationException;
import javax.swing.text.BoxView;
import javax.swing.text.ComponentView;
import javax.swing.text.Document;
import javax.swing.text.Element;
import javax.swing.text.IconView;
import javax.swing.text.LabelView;
import javax.swing.text.ParagraphView;
import javax.swing.text.SimpleAttributeSet;
import javax.swing.text.StyleConstants;
import javax.swing.text.StyledEditorKit;
import javax.swing.text.View;
import javax.swing.text.ViewFactory;

import com.blockforge.launcher.App;

public final class Console extends JTextPane {
    /**
     * How many lines the console keeps. A game that logs a lot would otherwise grow the console without end, and
     * every line costs the launcher memory. Older lines are still in the log file.
     */
    public static final int MAX_LINES = 5000;
    /** lines are dropped in batches, so a busy log doesn't trim (and lay the text out again) on every line */
    static final int TRIM_SLACK = 500;

    private final SimpleAttributeSet attrs = new SimpleAttributeSet();

    public Console() {
        this.setEditable(false);
        this.setEditorKit(new WrapEditorKit());
        this.setFont(App.THEME.getConsoleFont().deriveFont((float) UIManager.get("Console.fontSize")));
    }

    public Console setColor(Color c) {
        StyleConstants.setForeground(this.attrs, c);
        return this;
    }

    public Console setBold(boolean b) {
        StyleConstants.setBold(this.attrs, b);
        return this;
    }

    @Override
    public boolean getScrollableTracksViewportWidth() {
        return true;
    }

    public void write(String str) {
        try {
            this.getDocument().insertString(this.getDocument().getLength(), str, this.attrs);
            trimToLines(this.getDocument(), MAX_LINES, TRIM_SLACK);
            this.setCaretPosition(this.getDocument().getLength());
        } catch (BadLocationException ex) {
            ex.printStackTrace(System.err);
        }
    }

    /**
     * Removes the oldest lines once the document holds more than maxLines + slack finished lines, leaving maxLines.
     *
     * @return how many lines were removed
     */
    static int trimToLines(Document doc, int maxLines, int slack) throws BadLocationException {
        Element root = doc.getDefaultRootElement();
        // the last line is the one still being written to, so it doesn't count
        int finished = root.getElementCount() - 1;
        if (finished <= maxLines + slack) {
            return 0;
        }
        int remove = finished - maxLines;
        doc.remove(0, root.getElement(remove).getStartOffset());
        return remove;
    }
}

// https://stackoverflow.com/a/13375811
class WrapEditorKit extends StyledEditorKit {
    ViewFactory defaultFactory = new WrapColumnFactory();

    @Override
    public ViewFactory getViewFactory() {
        return defaultFactory;
    }

}

class WrapColumnFactory implements ViewFactory {
    @Override
    public View create(Element elem) {
        String kind = elem.getName();
        if (kind != null) {
            switch (kind) {
                case AbstractDocument.ContentElementName:
                    return new WrapLabelView(elem);
                case AbstractDocument.ParagraphElementName:
                    return new ParagraphView(elem);
                case AbstractDocument.SectionElementName:
                    return new BoxView(elem, View.Y_AXIS);
                case StyleConstants.ComponentElementName:
                    return new ComponentView(elem);
                case StyleConstants.IconElementName:
                    return new IconView(elem);
            }
        }

        // default to text display
        return new LabelView(elem);
    }
}

class WrapLabelView extends LabelView {
    public WrapLabelView(Element elem) {
        super(elem);
    }

    @Override
    public float getMinimumSpan(int axis) {
        switch (axis) {
            case View.X_AXIS:
                return 0;
            case View.Y_AXIS:
                return super.getMinimumSpan(axis);
            default:
                throw new IllegalArgumentException("Invalid axis: " + axis);
        }
    }

}
