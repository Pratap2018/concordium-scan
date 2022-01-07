namespace ConcordiumSdk.Types;

public class AccountAddress : Address
{
    /// <summary>
    /// Creates an instance from a 32 byte address (ie. excluding the version byte).
    /// </summary>
    public AccountAddress(byte[] bytes) : base(bytes)
    {
    }
    
    /// <summary>
    /// Creates an instance from a base58-check encoded string
    /// </summary>
    public AccountAddress(string base58CheckEncodedAddress) : base(base58CheckEncodedAddress)
    {
    }
}