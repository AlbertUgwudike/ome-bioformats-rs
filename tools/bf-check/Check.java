import loci.formats.*;
import loci.formats.meta.IMetadata;
import loci.formats.services.OMEXMLService;
import loci.common.services.ServiceFactory;
import loci.common.DebugTools;
import java.security.MessageDigest;

public class Check {
  public static void main(String[] args) throws Exception {
    DebugTools.setRootLevel("ERROR");
    OMEXMLService svc = new ServiceFactory().getInstance(OMEXMLService.class);
    IMetadata meta = svc.createOMEXMLMetadata();
    ImageReader r = new ImageReader();
    r.setMetadataStore(meta);
    r.setId(args[0]);
    System.out.println("reader=" + r.getReader().getClass().getSimpleName() + " series=" + r.getSeriesCount());
    for (int s = 0; s < r.getSeriesCount(); s++) {
      r.setSeries(s);
      System.out.println(" s" + s + " " + r.getSizeX() + "x" + r.getSizeY() + " Z" + r.getSizeZ() + " C" + r.getSizeC() + " T" + r.getSizeT()
        + " order=" + r.getDimensionOrder() + " type=" + FormatTools.getPixelTypeString(r.getPixelType()) + " rgb=" + r.getRGBChannelCount()
        + " name=" + meta.getImageName(s) + " psx=" + meta.getPixelsPhysicalSizeX(s) + " date=" + meta.getImageAcquisitionDate(s));
      for (int p = 0; p < r.getImageCount(); p++) {
        int[] zct = r.getZCTCoords(p);
        // hash in row strips to stay within int-sized buffers
        MessageDigest md = MessageDigest.getInstance("MD5");
        int rows = Math.max(1, (64 << 20) / (r.getSizeX() * FormatTools.getBytesPerPixel(r.getPixelType()) * r.getRGBChannelCount()));
        for (int y = 0; y < r.getSizeY(); y += rows) {
          int h = Math.min(rows, r.getSizeY() - y);
          byte[] b = r.openBytes(p, 0, y, r.getSizeX(), h);
          int spp = r.getRGBChannelCount(), bps = FormatTools.getBytesPerPixel(r.getPixelType());
          if (spp > 1 && !r.isInterleaved()) {
            // normalise planar RGB to interleaved so layouts compare equal
            byte[] o = new byte[b.length]; int n = b.length / spp / bps;
            for (int s2 = 0; s2 < spp; s2++) for (int i = 0; i < n; i++)
              System.arraycopy(b, (s2 * n + i) * bps, o, (i * spp + s2) * bps, bps);
            b = o;
          }
          md.update(b);
        }
        StringBuilder sb = new StringBuilder();
        for (byte b : md.digest()) sb.append(String.format("%02x", b));
        System.out.println("  plane " + p + " zct=" + zct[0] + "," + zct[1] + "," + zct[2] + " md5=" + sb);
      }
    }
    if (args.length > 1) {
      String xml = svc.getOMEXML(meta);
      System.out.println("OME-XML valid per Bio-Formats: " + svc.validateOMEXML(xml));
    }
    r.close();
  }
}
