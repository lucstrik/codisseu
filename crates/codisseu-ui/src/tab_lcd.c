/* Codisseu adapter to the public FreeType API. No JBR implementation code. */
#include <stdint.h>
#include <stdlib.h>
#include <limits.h>
#include <ft2build.h>
#include FT_FREETYPE_H
#include FT_LCD_FILTER_H
#include FT_OUTLINE_H
#include FT_MULTIPLE_MASTERS_H

typedef struct { FT_Library library; FT_Face face; } TabFace;

void codisseu_tab_face_drop(void *opaque) {
    TabFace *face = opaque;
    if (!face) return;
    if (face->face) FT_Done_Face(face->face);
    if (face->library) FT_Done_FreeType(face->library);
    free(face);
}

void *codisseu_tab_face_new(const uint8_t *font, size_t len) {
    if (!font || !len || len > LONG_MAX) return NULL;
    TabFace *face = calloc(1, sizeof(*face));
    if (!face) return NULL;
    if (FT_Init_FreeType(&face->library) ||
        FT_New_Memory_Face(face->library, font, (FT_Long)len, 0, &face->face) ||
        FT_Library_SetLcdFilter(face->library, FT_LCD_FILTER_DEFAULT)) {
        codisseu_tab_face_drop(face);
        return NULL;
    }
    return face;
}

static int render_face(void *opaque, const uint32_t *text, size_t count,
                            int size_26_6, int width, int height,
                            int cursor, int baseline, const uint8_t *foreground,
                            unsigned opacity, uint8_t *rgb, size_t len, int embolden, int clip_right) {
    TabFace *face = opaque;
    if (!face || !text || !rgb || !foreground || opacity > 255 || count > 128 || size_26_6 < 1 ||
        width < 1 || width > 4096 || height < 1 || height > 256 ||
        (size_t)width * (size_t)height > 65536 ||
        (size_t)width * (size_t)height * 3 != len) return 0;
    if (FT_Set_Char_Size(face->face, 0, size_26_6, 72, 72)) return 0;
    for (size_t i = 0; i < count; ++i) {
        if (!FT_Get_Char_Index(face->face, text[i]) ||
            FT_Load_Char(face->face, text[i], FT_LOAD_TARGET_LCD | FT_LOAD_NO_BITMAP)) return 0;
        if (embolden && (face->face->glyph->format != FT_GLYPH_FORMAT_OUTLINE ||
            FT_Outline_EmboldenXY(&face->face->glyph->outline, embolden, embolden))) return 0;
        if (FT_Render_Glyph(face->face->glyph, FT_RENDER_MODE_LCD)) return 0;
        FT_GlyphSlot glyph = face->face->glyph;
        FT_Bitmap *bitmap = &glyph->bitmap;
        if (bitmap->pixel_mode != FT_PIXEL_MODE_LCD || bitmap->width % 3 ||
            bitmap->width > 4096 * 3 || bitmap->rows > 256) return 0;
        int left = cursor + glyph->bitmap_left;
        int top = baseline - glyph->bitmap_top;
        int columns = (int)bitmap->width / 3;
        if (left < 0 || top < 0 || left + columns > width ||
            top + (int)bitmap->rows > height) return 0;
        for (unsigned y = 0; y < bitmap->rows; ++y) {
            const uint8_t *row = bitmap->pitch >= 0
                ? bitmap->buffer + y * bitmap->pitch
                : bitmap->buffer + (bitmap->rows - 1 - y) * -(bitmap->pitch);
            for (unsigned x = 0; x < bitmap->width; ++x) {
                if (left + (int)(x / 3) >= clip_right) continue;
                size_t at = ((size_t)(top + y) * width + left) * 3 + x;
                // Compose each glyph in order. Combining coverage first loses
                // the integer rounding of overlapping LCD filter fringes.
                unsigned alpha = (row[x] * opacity + 127) / 255;
                unsigned value = (foreground[x % 3] * alpha + 127) / 255
                    + (rgb[at] * (255 - alpha) + 127) / 255;
                rgb[at] = (uint8_t)(value > 255 ? 255 : value);
            }
        }
        cursor += (int)((glyph->advance.x + 32) / 64);
        if (cursor > width - 2) return 0;
    }
    return 1;
}

int codisseu_tab_face_render(void *opaque, const uint32_t *text, size_t count,
                            int size_26_6, int width, int height,
                            int cursor, int baseline, const uint8_t *foreground,
                            unsigned opacity, uint8_t *rgb, size_t len) {
    return render_face(opaque, text, count, size_26_6, width, height,
                       cursor, baseline, foreground, opacity, rgb, len, 0, width);
}

/* Signature uses its own regular face. Synthetic bold changes outlines, not advances. */
static int signature_prepare(TabFace *face, int size_26_6, int bold) {
    if (!face || size_26_6 < 1 || size_26_6 > 72 * 64 || (bold != 0 && bold != 1)) return 0;
    FT_MM_Var *variation = NULL;
    if (FT_Get_MM_Var(face->face, &variation) || !variation) return 0;
    if (variation->num_axis > 16) {
        FT_Done_MM_Var(face->library, variation);
        return 0;
    }
    FT_Fixed coordinates[16];
    int found = 0;
    for (unsigned i = 0; i < variation->num_axis; ++i) {
        coordinates[i] = variation->axis[i].def;
        if (variation->axis[i].tag == FT_MAKE_TAG('w', 'g', 'h', 't')) {
            coordinates[i] = 400 * 65536L;
            found = 1;
        }
    }
    int ok = found && !FT_Set_Var_Design_Coordinates(face->face, variation->num_axis, coordinates);
    FT_Done_MM_Var(face->library, variation);
    return ok && !FT_Set_Char_Size(face->face, 0, size_26_6, 72, 72);
}

int codisseu_signature_face_measure(void *opaque, const uint32_t *text, size_t count,
                                   int size_26_6, int bold, int *advance) {
    TabFace *face = opaque;
    if (!text || !advance || count > 128 || !signature_prepare(face, size_26_6, bold)) return 0;
    int total = 0;
    for (size_t i = 0; i < count; ++i) {
        if (!FT_Get_Char_Index(face->face, text[i]) ||
            FT_Load_Char(face->face, text[i], FT_LOAD_TARGET_LCD | FT_LOAD_NO_BITMAP)) return 0;
        FT_Pos raw = face->face->glyph->advance.x;
        if (raw < 0 || raw > 4096 * 64 || total > 4096 - (int)((raw + 32) / 64)) return 0;
        total += (int)((raw + 32) / 64);
    }
    *advance = total;
    return 1;
}

int codisseu_signature_face_render(void *opaque, const uint32_t *text, size_t count,
                                  int size_26_6, int bold, int width, int height,
                                  int cursor, int baseline, const uint8_t *foreground,
                                  uint8_t *rgb, size_t len, int clip_right) {
    TabFace *face = opaque;
    if (clip_right < 0 || clip_right > width) return 0;
    if (!signature_prepare(face, size_26_6, bold)) return 0;
    // The public API keeps rasterization independent of the reference runtime.
    // Strength size/32 is documented in the research source and verified at 13px.
    return render_face(opaque, text, count, size_26_6, width, height,
                       cursor, baseline, foreground, 255, rgb, len, bold ? size_26_6 / 32 : 0, clip_right);
}
