"""Independent FCM parser: dumps every field, records byte offsets, renders SVG."""
import struct, sys, json

class R:
    def __init__(self, b, off=0, end=None):
        self.b, self.o, self.end = b, off, len(b) if end is None else end
    def u8(self):  v = self.b[self.o]; self.o += 1; return v
    def u16(self): v = struct.unpack_from('<H', self.b, self.o)[0]; self.o += 2; return v
    def u32(self): v = struct.unpack_from('<I', self.b, self.o)[0]; self.o += 4; return v
    def i32(self): v = struct.unpack_from('<i', self.b, self.o)[0]; self.o += 4; return v
    def f32(self): v = struct.unpack_from('<f', self.b, self.o)[0]; self.o += 4; return v
    def raw(self, n): v = self.b[self.o:self.o+n]; self.o += n; return v
    def pt(self): return (self.i32(), self.i32())
    def u16str(self):
        n = self.u8(); return ''.join(chr(self.u16()) for _ in range(n))

def parse(b):
    r = R(b)
    out = {}
    out['variant'] = r.raw(4).decode()
    out['version'] = r.raw(4).decode()
    out['content_id'] = r.u32()
    hlen = r.u32(); hstart = r.o
    out['header_len'] = hlen
    out['short_name'] = r.raw(8).split(b'\0')[0].decode()
    out['long_name'] = r.u16str()
    out['author'] = r.u16str()
    out['copyright'] = r.u16str()
    out['thumb_block_w'] = r.u8(); out['thumb_block_h'] = r.u8()
    tl = r.u32(); out['thumb_off'] = r.o; out['thumb_len'] = tl
    thumb = r.raw(tl)
    out['generator_tag'] = r.raw(4).decode('latin1')
    out['generator_ver'] = r.u32()
    if r.o < hstart + hlen:
        out['print_to_cut'] = r.u32()
    assert r.o == hstart + hlen, (r.o, hstart + hlen)
    # cut data
    ft = r.u32(); out['file_type'] = hex(ft)
    out['mat_id'] = r.u32()
    out['cut_width'] = r.u32(); out['cut_height'] = r.u32()
    out['seam_allowance'] = r.u32()
    if ft == 0x38:
        out['align_needed'] = r.u32()
        n = r.u32(); out['align_marks'] = [r.pt() for _ in range(n)]
    # piece table
    n = r.u32(); offs = [r.u32() for _ in range(n)]
    total = r.u32()
    nid = r.u32(); ids = [r.u16() for _ in range(nid)]
    base = r.o
    out['piece_count'] = n; out['piece_ids'] = ids; out['piece_offsets'] = offs
    out['piece_data_base'] = base; out['piece_data_total'] = total
    pieces = []
    for i, off in enumerate(offs):
        pr = R(b, base + off)
        p = {'off': base + off}
        p['reserved8'] = pr.raw(8).hex()
        p['width'] = pr.u32(); p['height'] = pr.u32()
        ht = pr.u32()
        p['transform'] = [pr.f32() for _ in range(6)] if ht else None
        p['expansion_limit'] = pr.u32(); p['reduction_limit'] = pr.u32()
        p['restrictions'] = hex(pr.u32())
        ll = pr.u32(); p['label_raw'] = pr.raw(ll).hex()
        npaths = pr.u32(); paths = []
        for _ in range(npaths):
            plen = pr.u32(); pend = pr.o + plen
            q = R(b, pr.o, pend)
            tl_ = q.u32(); assert tl_ == 4
            tool = q.u32()
            oc = q.u32(); rc = q.u32(); rd = q.u32()
            path = {'tool': hex(tool), 'outline_count': oc, 'rhinestone_count': rc,
                    'rhinestone_diam': hex(rd)}
            if oc:
                path['start'] = q.pt()
                outlines = []
                for _ in range(oc):
                    tag = q.u32(); cnt = q.u32()
                    if tag == 0:
                        segs = [('L', q.pt()) for _ in range(cnt)]
                    elif tag == 1:
                        segs = [('C', q.pt(), q.pt(), q.pt()) for _ in range(cnt)]
                    else:
                        raise ValueError(f'bad outline tag {tag} at {q.o}')
                    outlines.append(segs)
                path['outlines'] = outlines
            path['rhinestones'] = [q.pt() for _ in range(rc)]
            assert q.o == pend, (q.o, pend)
            pr.o = pend
            paths.append(path)
        p['paths'] = paths
        p['end'] = pr.o
        nxt = base + (offs[i+1] if i + 1 < n else total)
        p['slack'] = nxt - pr.o
        pieces.append(p)
    out['pieces'] = pieces
    out['trailing'] = len(b) - (base + total)
    return out, thumb

def svg(out, path):
    parts = []
    xs, ys = [], []
    for p in out['pieces']:
        t = p['transform'] or [1, 0, 0, 1, 0, 0]
        a, bb, c, d, e, f = t
        def X(pt):
            x, y = pt
            return (a * x + c * y + e, bb * x + d * y + f)
        for pa in p['paths']:
            if 'start' not in pa: continue
            s = X(pa['start']); dd = [f'M{s[0]:.1f},{s[1]:.1f}']
            xs.append(s[0]); ys.append(s[1])
            for ol in pa['outlines']:
                for seg in ol:
                    if seg[0] == 'L':
                        q = X(seg[1]); dd.append(f'L{q[0]:.1f},{q[1]:.1f}'); xs.append(q[0]); ys.append(q[1])
                    else:
                        c1, c2, q = X(seg[1]), X(seg[2]), X(seg[3])
                        dd.append(f'C{c1[0]:.1f},{c1[1]:.1f} {c2[0]:.1f},{c2[1]:.1f} {q[0]:.1f},{q[1]:.1f}')
                        xs.append(q[0]); ys.append(q[1])
            col = 'red' if int(pa['tool'], 16) & 2 else 'blue'
            parts.append(f'<path d="{" ".join(dd)}" fill="none" stroke="{col}" stroke-width="20"/>')
    x0, y0, x1, y1 = min(xs), min(ys), max(xs), max(ys)
    with open(path, 'w') as fh:
        fh.write(f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="{x0-200} {y0-200} {x1-x0+400} {y1-y0+400}" '
                 f'width="{(x1-x0+400)/100:.2f}mm" height="{(y1-y0+400)/100:.2f}mm"><rect x="{x0-200}" y="{y0-200}" width="{x1-x0+400}" height="{y1-y0+400}" fill="white"/>'
                 + ''.join(parts) + '</svg>')
    return (x0, y0, x1, y1)

if __name__ == '__main__':
    b = open(sys.argv[1], 'rb').read()
    out, thumb = parse(b)
    od = sys.argv[2]
    open(f'{od}/thumb.bmp', 'wb').write(thumb)
    bbox = svg(out, f'{od}/render.svg')
    json.dump(out, open(f'{od}/dump.json', 'w'), indent=1, default=str)
    summ = {k: v for k, v in out.items() if k not in ('pieces', 'piece_ids', 'piece_offsets')}
    print(json.dumps(summ, indent=1))
    print('ids', out['piece_ids'][:20], '...')
    print('bbox (transformed units)', bbox)
