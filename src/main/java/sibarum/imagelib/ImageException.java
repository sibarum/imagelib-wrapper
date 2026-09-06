package sibarum.imagelib;

/**
 * A decode, a rasterisation or a load that did not happen — carrying the native decoder's own
 * message where there is one.
 *
 * <p>Unchecked, because there is nothing a caller can usefully do about a corrupt file at the call
 * site that it could not do better one level up: substitute a placeholder, or skip the asset.
 */
public class ImageException extends RuntimeException {

    private static final long serialVersionUID = 1L;

    public ImageException(String message) {
        super(message);
    }

    public ImageException(String message, Throwable cause) {
        super(message, cause);
    }
}
