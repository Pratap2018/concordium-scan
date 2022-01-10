using System.Linq;

namespace ConcordiumSdk.Types;

/// <summary>
/// A Memo is stored on chain as just an array of bytes.
/// Convention is to encode a text message as CBOR, but this is not enforced by nodes.  
/// </summary>
public class Memo
{
    private readonly byte[] _bytes;

    public Memo(byte[] bytes)
    {
        _bytes = bytes ?? throw new ArgumentNullException(nameof(bytes));
    }

    public static Memo FromHexString(string hexString)
    {
        var bytes = Convert.FromHexString(hexString);
        return new Memo(bytes);
    }
    
    public byte[] AsBytes => _bytes;
    public string AsHex => Convert.ToHexString(_bytes).ToLowerInvariant();

    public override bool Equals(object? obj)
    {
        if (ReferenceEquals(null, obj)) return false;
        if (ReferenceEquals(this, obj)) return true;
        if (obj.GetType() != GetType()) return false;
        
        var other = (Memo)obj;
        return _bytes.SequenceEqual(other._bytes);
    }

    public override int GetHashCode()
    {
        return _bytes.GetHashCode();
    }

    public static bool operator ==(Memo? left, Memo? right)
    {
        return Equals(left, right);
    }

    public static bool operator !=(Memo? left, Memo? right)
    {
        return !Equals(left, right);
    }
}